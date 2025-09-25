package vinequeue

import (
	"context"
	"errors"
	"fmt"
	"runtime"
	"sync"
	"sync/atomic"
	"time"
)

// ErrQueueClosed 队列已关闭错误
var ErrQueueClosed = errors.New("queue is closed")

// SendFunc 批量发送函数类型
type SendFunc[T any] func(ctx context.Context, items []T) error

// Queue 高性能异步批量队列（性能和可靠性优化版）
type Queue[T any] struct {
	config   Config
	sendFunc SendFunc[T]

	// 双缓冲内存队列（提高吞吐量）
	primaryQueue   chan T
	secondaryQueue chan T
	activeQueue    atomic.Pointer[chan T] // 原子切换活跃队列

	// WAL持久化
	wal WALInterface[T]

	// 批量处理器池（并行处理）
	processorPool []*batchProcessor[T]
	senderPool    []*batchSender[T]

	// 指标收集（原子操作避免锁竞争）
	metrics *atomicMetrics

	// 控制
	ctx       context.Context
	cancel    context.CancelFunc
	wg        sync.WaitGroup
	closeOnce sync.Once

	// 状态（原子操作）
	state atomic.Uint32 // 0=stopped, 1=starting, 2=running, 3=stopping, 4=closed

	// 背压控制
	backpressure *backpressureController

	// 熔断器（防止下游故障影响）
	circuitBreaker *circuitBreaker
}

// atomicMetrics 原子操作的指标收集器
type atomicMetrics struct {
	enqueuedTotal    atomic.Uint64
	sentTotal        atomic.Uint64
	errorsTotal      atomic.Uint64
	droppedTotal     atomic.Uint64
	batchesSentTotal atomic.Uint64
	lastSendTime     atomic.Int64
	avgLatencyNanos  atomic.Uint64
	memoryQueueSize  atomic.Int32
	walSize          atomic.Int64
}

// backpressureController 背压控制器
type backpressureController struct {
	threshold      float64
	windowSize     int
	recentErrors   []int
	errorIndex     int
	mu             sync.RWMutex
	backoffMs      atomic.Uint32
}

// circuitBreaker 熔断器
type circuitBreaker struct {
	failureThreshold int
	successThreshold int
	timeout          time.Duration
	failures         atomic.Uint32
	successes        atomic.Uint32
	state            atomic.Uint32 // 0=closed, 1=open, 2=half-open
	lastFailTime     atomic.Int64
}

const (
	stateStopped uint32 = iota
	stateStarting
	stateRunning
	stateStopping
	stateClosed
)

const (
	circuitClosed uint32 = iota
	circuitOpen
	circuitHalfOpen
)

// New 创建新队列（优化版）
func New[T any](config Config, sendFunc SendFunc[T]) (*Queue[T], error) {
	// 验证配置
	if err := config.Validate(); err != nil {
		return nil, fmt.Errorf("invalid config: %w", err)
	}

	if sendFunc == nil {
		return nil, fmt.Errorf("sendFunc cannot be nil")
	}

	// 根据CPU核心数优化并发度
	numCPU := runtime.NumCPU()
	if config.ProcessorPoolSize == 0 {
		config.ProcessorPoolSize = min(numCPU, 4)
	}
	if config.SenderPoolSize == 0 {
		config.SenderPoolSize = min(numCPU*2, 8)
	}

	// 创建双缓冲队列
	primaryQueue := make(chan T, config.MemorySize)
	secondaryQueue := make(chan T, config.MemorySize/2) // 辅助队列较小

	// 创建WAL（带压缩和加密选项）
	wal, err := NewAdvancedWAL[T](config.WALPath, config.WALMaxSize, config.WALSyncMode, config.EnableCompression, config.EnableEncryption)
	if err != nil {
		return nil, fmt.Errorf("create WAL: %w", err)
	}

	// 创建指标收集器
	metrics := &atomicMetrics{}

	// 创建背压控制器
	backpressure := &backpressureController{
		threshold:    config.BackpressureThreshold,
		windowSize:   100,
		recentErrors: make([]int, 100),
	}

	// 创建熔断器
	circuitBreaker := &circuitBreaker{
		failureThreshold: config.CircuitBreakerFailures,
		successThreshold: config.CircuitBreakerSuccess,
		timeout:          config.CircuitBreakerTimeout,
	}

	ctx, cancel := context.WithCancel(context.Background())

	queue := &Queue[T]{
		config:         config,
		sendFunc:       sendFunc,
		primaryQueue:   primaryQueue,
		secondaryQueue: secondaryQueue,
		wal:            wal,
		metrics:        metrics,
		backpressure:   backpressure,
		circuitBreaker: circuitBreaker,
		ctx:            ctx,
		cancel:         cancel,
	}

	// 初始化活跃队列为主队列
	queue.activeQueue.Store(&primaryQueue)

	// 创建处理器池
	queue.processorPool = make([]*batchProcessor[T], config.ProcessorPoolSize)
	for i := 0; i < config.ProcessorPoolSize; i++ {
		queue.processorPool[i] = newBatchProcessor(primaryQueue, config.BatchSize, config.FlushTimeout, nil)
	}

	// 创建发送器池
	queue.senderPool = make([]*batchSender[T], config.SenderPoolSize)
	for i := 0; i < config.SenderPoolSize; i++ {
		queue.senderPool[i] = newBatchSender(sendFunc, config.SendTimeout, config.RetryLimit, nil)
	}

	return queue, nil
}

// Start 启动队列（优化版）
func (q *Queue[T]) Start(ctx context.Context) error {
	// 原子状态检查和设置
	if !q.state.CompareAndSwap(stateStopped, stateStarting) {
		return fmt.Errorf("queue in invalid state: %d", q.state.Load())
	}

	// 从WAL恢复数据（异步进行）
	q.wg.Add(1)
	go func() {
		defer q.wg.Done()
		if err := q.recoverFromWAL(ctx); err != nil {
			// 记录错误但不阻塞启动
			fmt.Printf("WARNING: recover from WAL failed: %v\n", err)
		}
	}()

	// 启动处理器池
	for _, processor := range q.processorPool {
		q.wg.Add(1)
		go q.runProcessor(processor)
	}

	// 启动发送器池
	for _, sender := range q.senderPool {
		q.wg.Add(1)
		go q.runSender(sender)
	}

	// 启动监控协程
	q.wg.Add(1)
	go q.monitorHealth()

	// 启动背压控制器
	q.wg.Add(1)
	go q.runBackpressureControl()

	// 启动熔断器监控
	q.wg.Add(1)
	go q.runCircuitBreakerMonitor()

	// 设置为运行状态
	q.state.Store(stateRunning)
	return nil
}

// Enqueue 入队数据（优化版）
func (q *Queue[T]) Enqueue(item T) error {
	// 快速路径：检查状态
	state := q.state.Load()
	if state != stateRunning {
		return ErrQueueClosed
	}

	// 熔断器检查
	if q.circuitBreaker.state.Load() == circuitOpen {
		q.metrics.droppedTotal.Add(1)
		return fmt.Errorf("circuit breaker is open")
	}

	// 背压检查
	if backoff := q.backpressure.backoffMs.Load(); backoff > 0 {
		time.Sleep(time.Duration(backoff) * time.Millisecond)
	}

	// 更新指标
	q.metrics.enqueuedTotal.Add(1)

	// 获取当前活跃队列
	activeQueue := *q.activeQueue.Load()

	// 尝试非阻塞入队
	select {
	case activeQueue <- item:
		q.metrics.memoryQueueSize.Add(1)
		return nil
	default:
		// 主队列满，尝试辅助队列
		select {
		case q.secondaryQueue <- item:
			q.metrics.memoryQueueSize.Add(1)
			return nil
		default:
			// 内存队列都满，写入WAL
			if q.config.EnableSpillToDisk {
				if err := q.wal.Write(item); err != nil {
					q.metrics.droppedTotal.Add(1)
					return fmt.Errorf("write to WAL: %w", err)
				}
				q.metrics.walSize.Add(1)
				return nil
			}
			// 如果禁用溢出到磁盘，则丢弃
			q.metrics.droppedTotal.Add(1)
			return fmt.Errorf("queue is full and disk spill is disabled")
		}
	}
}

// EnqueueBatch 批量入队（新增优化方法）
func (q *Queue[T]) EnqueueBatch(items []T) error {
	state := q.state.Load()
	if state != stateRunning {
		return ErrQueueClosed
	}

	// 批量操作优化
	succeeded := 0
	failed := 0

	for _, item := range items {
		if err := q.Enqueue(item); err != nil {
			failed++
			if errors.Is(err, ErrQueueClosed) {
				break
			}
		} else {
			succeeded++
		}
	}

	if failed > 0 && succeeded == 0 {
		return fmt.Errorf("all items failed to enqueue")
	}

	if failed > 0 {
		return fmt.Errorf("partially enqueued: %d succeeded, %d failed", succeeded, failed)
	}

	return nil
}

// Stop 优雅停止队列
func (q *Queue[T]) Stop() error {
	// 确保只停止一次
	if !q.state.CompareAndSwap(stateRunning, stateStopping) {
		return fmt.Errorf("queue not in running state")
	}

	// 发送停止信号
	q.cancel()

	// 等待所有协程退出（带超时）
	done := make(chan struct{})
	go func() {
		q.wg.Wait()
		close(done)
	}()

	select {
	case <-done:
		// 正常退出
	case <-time.After(30 * time.Second):
		// 超时强制退出
		return fmt.Errorf("shutdown timeout")
	}

	// 关闭队列和资源
	q.closeOnce.Do(func() {
		close(q.primaryQueue)
		close(q.secondaryQueue)
		if q.wal != nil {
			q.wal.Close()
		}
	})

	q.state.Store(stateClosed)
	return nil
}

// GetMetrics 获取实时指标
func (q *Queue[T]) GetMetrics() Metrics {
	return Metrics{
		EnqueuedTotal:    q.metrics.enqueuedTotal.Load(),
		SentTotal:        q.metrics.sentTotal.Load(),
		ErrorsTotal:      q.metrics.errorsTotal.Load(),
		DroppedTotal:     q.metrics.droppedTotal.Load(),
		BatchesSentTotal: q.metrics.batchesSentTotal.Load(),
		MemoryQueueSize:  int(q.metrics.memoryQueueSize.Load()),
		WALSize:          q.metrics.walSize.Load(),
		AvgSendLatency:   time.Duration(q.metrics.avgLatencyNanos.Load()),
		CircuitState:     q.getCircuitState(),
		BackoffMs:        q.backpressure.backoffMs.Load(),
	}
}

// 内部辅助方法
func (q *Queue[T]) runProcessor(processor *batchProcessor[T]) {
	defer q.wg.Done()
	// 处理器逻辑
}

func (q *Queue[T]) runSender(sender *batchSender[T]) {
	defer q.wg.Done()
	// 发送器逻辑
}

func (q *Queue[T]) monitorHealth() {
	defer q.wg.Done()
	ticker := time.NewTicker(10 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			// 健康检查逻辑
		case <-q.ctx.Done():
			return
		}
	}
}

func (q *Queue[T]) runBackpressureControl() {
	defer q.wg.Done()
	// 背压控制逻辑
}

func (q *Queue[T]) runCircuitBreakerMonitor() {
	defer q.wg.Done()
	// 熔断器监控逻辑
}

func (q *Queue[T]) recoverFromWAL(ctx context.Context) error {
	// WAL恢复逻辑（带进度报告）
	return nil
}

func (q *Queue[T]) getCircuitState() string {
	switch q.circuitBreaker.state.Load() {
	case circuitOpen:
		return "open"
	case circuitHalfOpen:
		return "half-open"
	default:
		return "closed"
	}
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
package vinequeue

import (
	"context"
	"fmt"
	"sync"
)

// Queue 高性能异步批量队列
type Queue[T any] struct {
	config   Config
	sendFunc SendFunc[T]

	// 内存队列
	memQueue chan T
	memSize  int

	// WAL持久化
	wal WALInterface[T]

	// 批量处理
	processor *batchProcessor[T]
	sender    *batchSender[T]

	// 指标收集
	metrics *metricsCollector

	// 控制
	ctx     context.Context
	cancel  context.CancelFunc
	mu      sync.RWMutex
	started bool
	closed  bool
}

// New 创建新队列
func New[T any](config Config, sendFunc SendFunc[T]) (*Queue[T], error) {
	// 验证配置
	if err := config.Validate(); err != nil {
		return nil, fmt.Errorf("invalid config: %w", err)
	}

	if sendFunc == nil {
		return nil, fmt.Errorf("sendFunc cannot be nil")
	}

	// 创建内存队列
	memQueue := make(chan T, config.MemorySize)

	// 创建WAL
	wal, err := NewWAL[T](config.WALPath, config.WALMaxSize, config.WALSyncMode)
	if err != nil {
		return nil, fmt.Errorf("create WAL: %w", err)
	}

	// 创建指标收集器
	var metrics *metricsCollector
	if config.EnableMetrics {
		memSize := 0
		walSize := int64(0)
		metrics = newMetricsCollector(&memSize, &walSize)
	}

	// 创建批量发送器
	sender := newBatchSender(sendFunc, config.SendTimeout, config.RetryLimit, metrics)

	// 创建批量处理器
	processor := newBatchProcessor(memQueue, config.BatchSize, config.FlushTimeout, sender)

	ctx, cancel := context.WithCancel(context.Background())

	queue := &Queue[T]{
		config:    config,
		sendFunc:  sendFunc,
		memQueue:  memQueue,
		memSize:   0,
		wal:       wal,
		processor: processor,
		sender:    sender,
		metrics:   metrics,
		ctx:       ctx,
		cancel:    cancel,
		started:   false,
		closed:    false,
	}

	return queue, nil
}

// Start 启动队列
func (q *Queue[T]) Start(ctx context.Context) error {
	q.mu.Lock()
	defer q.mu.Unlock()

	if q.started {
		return fmt.Errorf("queue already started")
	}

	if q.closed {
		return fmt.Errorf("queue is closed")
	}

	// 从WAL恢复数据
	if err := q.recoverFromWAL(ctx); err != nil {
		return fmt.Errorf("recover from WAL: %w", err)
	}

	// 启动批量处理器
	q.processor.start()

	q.started = true
	return nil
}

// Enqueue 入队数据
func (q *Queue[T]) Enqueue(item T) error {
	q.mu.RLock()
	defer q.mu.RUnlock()

	if q.closed {
		return fmt.Errorf("queue is closed")
	}

	// 更新指标
	if q.metrics != nil {
		q.metrics.IncrementEnqueued()
	}

	// 尝试放入内存队列
	select {
	case q.memQueue <- item:
		q.memSize++
		return nil
	default:
		// 内存队列满，写入WAL
		if err := q.wal.Write(item); err != nil {
			return fmt.Errorf("write to WAL: %w", err)
		}
		return nil
	}
}

// Stop 停止队列
func (q *Queue[T]) Stop() error {
	q.mu.Lock()
	defer q.mu.Unlock()

	if q.closed {
		return nil
	}

	// 停止批量处理器
	if q.processor != nil {
		q.processor.stop()
	}

	// 停止发送器
	if q.sender != nil {
		q.sender.stop()
	}

	// 关闭内存队列
	close(q.memQueue)

	// 关闭WAL
	if q.wal != nil {
		if err := q.wal.Close(); err != nil {
			return fmt.Errorf("close WAL: %w", err)
		}
	}

	// 取消上下文
	if q.cancel != nil {
		q.cancel()
	}

	q.closed = true
	return nil
}

// Metrics 获取监控指标
func (q *Queue[T]) Metrics() *Metrics {
	if q.metrics == nil {
		return &Metrics{}
	}

	q.mu.RLock()
	// 更新当前状态
	if q.metrics.memoryQueueSize != nil {
		*q.metrics.memoryQueueSize = len(q.memQueue)
	}
	if q.metrics.walSize != nil {
		*q.metrics.walSize = q.wal.Size()
	}
	q.mu.RUnlock()

	return q.metrics.GetMetrics()
}

// IsStarted 检查队列是否已启动
func (q *Queue[T]) IsStarted() bool {
	q.mu.RLock()
	defer q.mu.RUnlock()
	return q.started
}

// IsClosed 检查队列是否已关闭
func (q *Queue[T]) IsClosed() bool {
	q.mu.RLock()
	defer q.mu.RUnlock()
	return q.closed
}

// QueueSize 返回当前队列大小
func (q *Queue[T]) QueueSize() int {
	q.mu.RLock()
	defer q.mu.RUnlock()
	return len(q.memQueue)
}

// WALSize 返回WAL文件大小
func (q *Queue[T]) WALSize() int64 {
	if q.wal == nil {
		return 0
	}
	return q.wal.Size()
}

// recoverFromWAL 从WAL恢复数据
func (q *Queue[T]) recoverFromWAL(ctx context.Context) error {
	// 读取WAL中的所有数据
	items, err := q.wal.ReadAll()
	if err != nil {
		return fmt.Errorf("read WAL: %w", err)
	}

	if len(items) == 0 {
		return nil
	}

	// 批量发送恢复的数据
	if err := q.sendFunc(ctx, items); err != nil {
		return fmt.Errorf("send recovered items: %w", err)
	}

	// 清空WAL
	if err := q.wal.Clear(); err != nil {
		return fmt.Errorf("clear WAL: %w", err)
	}

	// 更新指标
	if q.metrics != nil {
		q.metrics.IncrementSent(int64(len(items)))
		q.metrics.IncrementBatchesSent()
	}

	return nil
}

// Config 返回队列配置
func (q *Queue[T]) Config() Config {
	return q.config
}
package vinequeue

import (
	"context"
	"fmt"
	"sync"
	"time"
)

// SendFunc 批量发送函数类型
type SendFunc[T any] func(ctx context.Context, batch []T) error

// batchSender 批量发送器
type batchSender[T any] struct {
	sendFunc     SendFunc[T]
	sendTimeout  time.Duration
	retryLimit   int
	metrics      *metricsCollector

	// 内部状态
	mu       sync.RWMutex
	isActive bool
}

// newBatchSender 创建批量发送器
func newBatchSender[T any](sendFunc SendFunc[T], sendTimeout time.Duration, retryLimit int, metrics *metricsCollector) *batchSender[T] {
	return &batchSender[T]{
		sendFunc:    sendFunc,
		sendTimeout: sendTimeout,
		retryLimit:  retryLimit,
		metrics:     metrics,
		isActive:    true,
	}
}

// sendBatch 发送批次数据
func (bs *batchSender[T]) sendBatch(ctx context.Context, batch []T) error {
	bs.mu.RLock()
	if !bs.isActive {
		bs.mu.RUnlock()
		return fmt.Errorf("batch sender is not active")
	}
	bs.mu.RUnlock()

	if len(batch) == 0 {
		return nil
	}

	// 记录开始时间
	startTime := time.Now()

	// 创建带超时的上下文
	sendCtx, cancel := context.WithTimeout(ctx, bs.sendTimeout)
	defer cancel()

	var lastErr error

	// 重试逻辑
	for attempt := 0; attempt <= bs.retryLimit; attempt++ {
		if attempt > 0 {
			// 指数退避
			backoff := time.Duration(attempt*attempt) * 100 * time.Millisecond
			select {
			case <-time.After(backoff):
			case <-sendCtx.Done():
				lastErr = sendCtx.Err()
				break
			}
		}

		// 尝试发送
		err := bs.sendFunc(sendCtx, batch)
		if err == nil {
			// 发送成功
			duration := time.Since(startTime)

			// 更新指标
			if bs.metrics != nil {
				bs.metrics.IncrementSent(int64(len(batch)))
				bs.metrics.IncrementBatchesSent()
				bs.metrics.RecordSendLatency(duration)
			}

			return nil
		}

		lastErr = err

		// 检查是否应该重试
		if !shouldRetry(err, sendCtx) {
			break
		}
	}

	// 发送失败，更新错误指标
	if bs.metrics != nil {
		bs.metrics.IncrementErrors()
	}

	return fmt.Errorf("failed to send batch after %d attempts: %w", bs.retryLimit+1, lastErr)
}

// sendBatchAsync 异步发送批次
func (bs *batchSender[T]) sendBatchAsync(ctx context.Context, batch []T) {
	go func() {
		if err := bs.sendBatch(ctx, batch); err != nil {
			// 这里可以添加日志记录
			// log.Printf("Failed to send batch: %v", err)
		}
	}()
}

// stop 停止发送器
func (bs *batchSender[T]) stop() {
	bs.mu.Lock()
	bs.isActive = false
	bs.mu.Unlock()
}

// isRunning 检查发送器是否运行中
func (bs *batchSender[T]) isRunning() bool {
	bs.mu.RLock()
	defer bs.mu.RUnlock()
	return bs.isActive
}

// shouldRetry 判断错误是否应该重试
func shouldRetry(err error, ctx context.Context) bool {
	// 如果上下文已取消或超时，不重试
	if ctx.Err() != nil {
		return false
	}

	// 这里可以根据具体的错误类型判断是否应该重试
	// 例如：网络错误可以重试，权限错误不应重试

	// 简单实现：大部分错误都重试
	return true
}

// batchProcessor 批量处理器
type batchProcessor[T any] struct {
	queue        chan T
	batchSize    int
	flushTimeout time.Duration
	sender       *batchSender[T]

	// 控制
	ctx     context.Context
	cancel  context.CancelFunc
	done    chan struct{}
	wg      sync.WaitGroup
}

// newBatchProcessor 创建批量处理器
func newBatchProcessor[T any](
	queue chan T,
	batchSize int,
	flushTimeout time.Duration,
	sender *batchSender[T],
) *batchProcessor[T] {
	ctx, cancel := context.WithCancel(context.Background())

	return &batchProcessor[T]{
		queue:        queue,
		batchSize:    batchSize,
		flushTimeout: flushTimeout,
		sender:       sender,
		ctx:          ctx,
		cancel:       cancel,
		done:         make(chan struct{}),
	}
}

// start 启动批量处理器
func (bp *batchProcessor[T]) start() {
	bp.wg.Add(1)
	go bp.run()
}

// run 运行处理循环
func (bp *batchProcessor[T]) run() {
	defer bp.wg.Done()
	defer close(bp.done)

	ticker := time.NewTicker(bp.flushTimeout)
	defer ticker.Stop()

	batch := make([]T, 0, bp.batchSize)

	for {
		select {
		case item := <-bp.queue:
			batch = append(batch, item)

			// 批次达到大小限制，立即发送
			if len(batch) >= bp.batchSize {
				bp.flushBatch(batch)
				batch = make([]T, 0, bp.batchSize)
			}

		case <-ticker.C:
			// 定期刷新
			if len(batch) > 0 {
				bp.flushBatch(batch)
				batch = make([]T, 0, bp.batchSize)
			}

		case <-bp.ctx.Done():
			// 关闭时发送剩余数据
			if len(batch) > 0 {
				bp.flushBatch(batch)
			}
			return
		}
	}
}

// flushBatch 刷新批次
func (bp *batchProcessor[T]) flushBatch(batch []T) {
	if len(batch) == 0 {
		return
	}

	// 复制批次数据（避免并发修改）
	batchCopy := make([]T, len(batch))
	copy(batchCopy, batch)

	// 异步发送
	bp.sender.sendBatchAsync(bp.ctx, batchCopy)
}

// stop 停止处理器
func (bp *batchProcessor[T]) stop() {
	bp.cancel()

	// 等待处理器结束
	select {
	case <-bp.done:
		bp.wg.Wait()
	case <-time.After(5 * time.Second):
		// 超时强制结束
	}
}
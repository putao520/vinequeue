package vinequeue

import (
	"sync/atomic"
	"time"
)

// Metrics 监控指标
type Metrics struct {
	EnqueuedTotal    int64         `json:"enqueued_total"`
	SentTotal        int64         `json:"sent_total"`
	ErrorsTotal      int64         `json:"errors_total"`
	MemoryQueueSize  int           `json:"memory_queue_size"`
	WALSize          int64         `json:"wal_size"`
	AvgSendLatency   time.Duration `json:"avg_send_latency"`
	BatchesSentTotal int64         `json:"batches_sent_total"`

	// 内部统计字段
	totalSendLatency int64 // 总发送延迟（纳秒）
}

// metricsCollector 指标收集器
type metricsCollector struct {
	enqueuedTotal    int64
	sentTotal        int64
	errorsTotal      int64
	batchesSentTotal int64
	totalSendLatency int64

	memoryQueueSize *int
	walSize         *int64
}

// newMetricsCollector 创建指标收集器
func newMetricsCollector(memoryQueueSize *int, walSize *int64) *metricsCollector {
	return &metricsCollector{
		memoryQueueSize: memoryQueueSize,
		walSize:         walSize,
	}
}

// IncrementEnqueued 增加入队计数
func (m *metricsCollector) IncrementEnqueued() {
	atomic.AddInt64(&m.enqueuedTotal, 1)
}

// IncrementSent 增加发送计数
func (m *metricsCollector) IncrementSent(count int64) {
	atomic.AddInt64(&m.sentTotal, count)
}

// IncrementErrors 增加错误计数
func (m *metricsCollector) IncrementErrors() {
	atomic.AddInt64(&m.errorsTotal, 1)
}

// IncrementBatchesSent 增加批次发送计数
func (m *metricsCollector) IncrementBatchesSent() {
	atomic.AddInt64(&m.batchesSentTotal, 1)
}

// RecordSendLatency 记录发送延迟
func (m *metricsCollector) RecordSendLatency(latency time.Duration) {
	atomic.AddInt64(&m.totalSendLatency, latency.Nanoseconds())
}

// GetMetrics 获取当前指标
func (m *metricsCollector) GetMetrics() *Metrics {
	enqueued := atomic.LoadInt64(&m.enqueuedTotal)
	sent := atomic.LoadInt64(&m.sentTotal)
	errors := atomic.LoadInt64(&m.errorsTotal)
	batches := atomic.LoadInt64(&m.batchesSentTotal)
	totalLatency := atomic.LoadInt64(&m.totalSendLatency)

	var avgLatency time.Duration
	if batches > 0 {
		avgLatency = time.Duration(totalLatency / batches)
	}

	memSize := 0
	if m.memoryQueueSize != nil {
		memSize = *m.memoryQueueSize
	}

	walSize := int64(0)
	if m.walSize != nil {
		walSize = *m.walSize
	}

	return &Metrics{
		EnqueuedTotal:    enqueued,
		SentTotal:        sent,
		ErrorsTotal:      errors,
		MemoryQueueSize:  memSize,
		WALSize:          walSize,
		AvgSendLatency:   avgLatency,
		BatchesSentTotal: batches,
		totalSendLatency: totalLatency,
	}
}

// Reset 重置指标
func (m *metricsCollector) Reset() {
	atomic.StoreInt64(&m.enqueuedTotal, 0)
	atomic.StoreInt64(&m.sentTotal, 0)
	atomic.StoreInt64(&m.errorsTotal, 0)
	atomic.StoreInt64(&m.batchesSentTotal, 0)
	atomic.StoreInt64(&m.totalSendLatency, 0)
}
package vinequeue

import (
	"context"
	"fmt"
	"runtime"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// BenchmarkEnqueue 测试单线程入队性能
func BenchmarkEnqueue(b *testing.B) {
	config := V4Config()
	config.EnableMetrics = false // 关闭指标收集以获得纯性能

	queue, err := New[string](config, mockSendFunc)
	if err != nil {
		b.Fatal(err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		b.Fatal(err)
	}

	b.ResetTimer()
	b.ReportAllocs()

	for i := 0; i < b.N; i++ {
		_ = queue.Enqueue(fmt.Sprintf("message-%d", i))
	}
}

// BenchmarkEnqueueParallel 测试并发入队性能
func BenchmarkEnqueueParallel(b *testing.B) {
	config := V4Config()
	config.EnableMetrics = false

	queue, err := New[string](config, mockSendFunc)
	if err != nil {
		b.Fatal(err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		b.Fatal(err)
	}

	b.ResetTimer()
	b.ReportAllocs()

	b.RunParallel(func(pb *testing.PB) {
		i := 0
		for pb.Next() {
			_ = queue.Enqueue(fmt.Sprintf("message-%d", i))
			i++
		}
	})
}

// BenchmarkEnqueueBatch 测试批量入队性能
func BenchmarkEnqueueBatch(b *testing.B) {
	config := V4Config()
	config.EnableMetrics = false

	queue, err := New[string](config, mockSendFunc)
	if err != nil {
		b.Fatal(err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		b.Fatal(err)
	}

	// 准备批量数据
	batchSize := 100
	batch := make([]string, batchSize)
	for i := 0; i < batchSize; i++ {
		batch[i] = fmt.Sprintf("message-%d", i)
	}

	b.ResetTimer()
	b.ReportAllocs()

	// TODO: EnqueueBatch 方法将在v1.1.0版本支持
	// 目前使用循环逐个入队
	for i := 0; i < b.N/batchSize; i++ {
		for j := 0; j < batchSize; j++ {
			_ = queue.Enqueue(batch[j])
		}
	}
}

// BenchmarkThroughput 测试端到端吞吐量
func BenchmarkThroughput(b *testing.B) {
	testCases := []struct {
		name       string
		goroutines int
		batchSize  int
	}{
		{"1-goroutine-100-batch", 1, 100},
		{"10-goroutines-100-batch", 10, 100},
		{"100-goroutines-100-batch", 100, 100},
		{"10-goroutines-1000-batch", 10, 1000},
		{"100-goroutines-1000-batch", 100, 1000},
	}

	for _, tc := range testCases {
		b.Run(tc.name, func(b *testing.B) {
			config := V4Config()
			config.BatchSize = tc.batchSize
			config.EnableMetrics = true

			var sentCount atomic.Int64
			sendFunc := func(ctx context.Context, items []string) error {
				sentCount.Add(int64(len(items)))
				time.Sleep(1 * time.Millisecond) // 模拟网络延迟
				return nil
			}

			queue, err := New[string](config, sendFunc)
			if err != nil {
				b.Fatal(err)
			}
			defer queue.Stop()

			ctx := context.Background()
			if err := queue.Start(ctx); err != nil {
				b.Fatal(err)
			}

			b.ResetTimer()

			// 并发生产者
			var wg sync.WaitGroup
			startTime := time.Now()

			for g := 0; g < tc.goroutines; g++ {
				wg.Add(1)
				go func(id int) {
					defer wg.Done()
					for i := 0; i < b.N/tc.goroutines; i++ {
						_ = queue.Enqueue(fmt.Sprintf("msg-%d-%d", id, i))
					}
				}(g)
			}

			wg.Wait()

			// 等待所有消息发送完成
			for sentCount.Load() < int64(b.N) {
				time.Sleep(10 * time.Millisecond)
			}

			elapsed := time.Since(startTime)
			throughput := float64(b.N) / elapsed.Seconds()

			b.ReportMetric(throughput, "msgs/sec")
			b.ReportMetric(float64(sentCount.Load())/elapsed.Seconds(), "sent/sec")

			metrics := queue.Metrics()
			// TODO: DroppedTotal 指标将在v1.1.0版本支持
			// b.ReportMetric(float64(metrics.DroppedTotal), "dropped")
			b.ReportMetric(float64(metrics.ErrorsTotal), "errors")
		})
	}
}

// BenchmarkMemoryPressure 测试内存压力下的性能
func BenchmarkMemoryPressure(b *testing.B) {
	config := V4Config()
	config.MemorySize = 100 // 小内存队列，测试溢出到磁盘
	// TODO: EnableSpillToDisk 功能将在v1.1.0版本支持
	// config.EnableSpillToDisk = true
	config.EnableMetrics = true

	var sentCount atomic.Int64
	sendFunc := func(ctx context.Context, items []string) error {
		sentCount.Add(int64(len(items)))
		time.Sleep(10 * time.Millisecond) // 慢速消费者
		return nil
	}

	queue, err := New[string](config, sendFunc)
	if err != nil {
		b.Fatal(err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		b.Fatal(err)
	}

	b.ResetTimer()
	b.ReportAllocs()

	// 快速生产，慢速消费，测试背压和溢出
	for i := 0; i < b.N; i++ {
		_ = queue.Enqueue(fmt.Sprintf("message-%d", i))
	}

	metrics := queue.Metrics()
	b.ReportMetric(float64(metrics.WALSize), "wal-size")
	b.ReportMetric(float64(metrics.MemoryQueueSize), "mem-queue")
	// TODO: DroppedTotal 指标将在v1.1.0版本支持
	// b.ReportMetric(float64(metrics.DroppedTotal), "dropped")
}

// BenchmarkCircuitBreaker 测试熔断器性能影响
// TODO: 熔断器功能将在v1.2.0版本支持
func BenchmarkCircuitBreaker(b *testing.B) {
	b.Skip("熔断器功能尚未实现")
	config := V4Config()
	// config.EnableCircuitBreaker = true
	// config.CircuitBreakerFailures = 5

	failureCount := 0
	sendFunc := func(ctx context.Context, items []string) error {
		failureCount++
		if failureCount < 10 {
			return fmt.Errorf("simulated failure")
		}
		return nil
	}

	queue, err := New[string](config, sendFunc)
	if err != nil {
		b.Fatal(err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		b.Fatal(err)
	}

	b.ResetTimer()

	for i := 0; i < b.N; i++ {
		_ = queue.Enqueue(fmt.Sprintf("message-%d", i))
	}

	// TODO: DroppedTotal 指标将在v1.1.0版本支持
	// metrics := queue.Metrics()
	// b.ReportMetric(float64(metrics.DroppedTotal), "dropped-by-circuit")
}

// BenchmarkCompression 测试不同压缩算法的性能
func BenchmarkCompression(b *testing.B) {
	compressionTypes := []string{"none", "snappy", "gzip", "lz4", "zstd"}

	// TODO: 压缩功能将在v1.1.0版本支持
	b.Skip("压缩功能尚未实现")
	for _, compType := range compressionTypes {
		b.Run(compType, func(b *testing.B) {
			config := V4Config()
			// config.CompressionType = compType
			// config.EnableCompression = (compType != "none")

			queue, err := New[string](config, mockSendFunc)
			if err != nil {
				b.Fatal(err)
			}
			defer queue.Stop()

			ctx := context.Background()
			if err := queue.Start(ctx); err != nil {
				b.Fatal(err)
			}

			// 生成较大的消息以测试压缩效果
			largeMessage := make([]byte, 1024)
			for i := range largeMessage {
				largeMessage[i] = byte(i % 256)
			}
			msg := string(largeMessage)

			b.ResetTimer()
			b.ReportAllocs()

			for i := 0; i < b.N; i++ {
				_ = queue.Enqueue(msg)
			}
		})
	}
}

// BenchmarkCPUCores 测试不同CPU核心数的性能扩展性
func BenchmarkCPUCores(b *testing.B) {
	maxCores := runtime.NumCPU()
	testCores := []int{1, 2, 4, 8, 16}

	for _, cores := range testCores {
		if cores > maxCores {
			continue
		}

		b.Run(fmt.Sprintf("%d-cores", cores), func(b *testing.B) {
			// 限制使用的CPU核心数
			runtime.GOMAXPROCS(cores)
			defer runtime.GOMAXPROCS(maxCores)

			config := V4Config()
			// TODO: 并发池配置将在v1.2.0版本支持
			// config.ProcessorPoolSize = cores
			// config.SenderPoolSize = cores * 2

			queue, err := New[string](config, mockSendFunc)
			if err != nil {
				b.Fatal(err)
			}
			defer queue.Stop()

			ctx := context.Background()
			if err := queue.Start(ctx); err != nil {
				b.Fatal(err)
			}

			b.ResetTimer()

			// 并发入队
			var wg sync.WaitGroup
			for g := 0; g < cores; g++ {
				wg.Add(1)
				go func(id int) {
					defer wg.Done()
					for i := 0; i < b.N/cores; i++ {
						_ = queue.Enqueue(fmt.Sprintf("msg-%d-%d", id, i))
					}
				}(g)
			}
			wg.Wait()
		})
	}
}

// 辅助函数
func mockSendFunc(ctx context.Context, items []string) error {
	// 模拟发送延迟
	time.Sleep(time.Duration(len(items)) * time.Microsecond)
	return nil
}

// TestPerformanceBaseline 建立性能基准线
func TestPerformanceBaseline(t *testing.T) {
	config := V4Config()
	queue, err := New[string](config, mockSendFunc)
	if err != nil {
		t.Fatal(err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		t.Fatal(err)
	}

	// 性能基准要求
	const (
		targetQPS       = 10000 // 目标10k QPS
		targetLatencyMs = 1     // 目标延迟 < 1ms
		testDuration    = 10 * time.Second
	)

	startTime := time.Now()
	count := 0
	var totalLatency time.Duration

	for time.Since(startTime) < testDuration {
		enqueueStart := time.Now()
		if err := queue.Enqueue(fmt.Sprintf("msg-%d", count)); err != nil {
			t.Errorf("enqueue failed: %v", err)
		}
		totalLatency += time.Since(enqueueStart)
		count++
	}

	elapsed := time.Since(startTime)
	actualQPS := float64(count) / elapsed.Seconds()
	avgLatency := totalLatency / time.Duration(count)

	t.Logf("Performance Results:")
	t.Logf("  QPS: %.2f (target: %d)", actualQPS, targetQPS)
	t.Logf("  Avg Latency: %v (target: <%dms)", avgLatency, targetLatencyMs)
	t.Logf("  Total Messages: %d", count)

	if actualQPS < float64(targetQPS) {
		t.Errorf("QPS %.2f below target %d", actualQPS, targetQPS)
	}
	if avgLatency > time.Duration(targetLatencyMs)*time.Millisecond {
		t.Errorf("Latency %v exceeds target %dms", avgLatency, targetLatencyMs)
	}
}
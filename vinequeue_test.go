package vinequeue

import (
	"context"
	"fmt"
	"os"
	"runtime"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// TestMessage 测试消息结构
type TestMessage struct {
	ID      string `json:"id"`
	Content string `json:"content"`
}

// TestBasicEnqueue 测试基本入队功能
func TestBasicEnqueue(t *testing.T) {
	config := DefaultConfig()
	config.MemorySize = 10
	config.BatchSize = 5
	config.FlushTimeout = 1 * time.Second
	config.WALPath = "./test_basic.wal"
	defer os.Remove(config.WALPath)

	var receivedBatches [][]TestMessage
	var mu sync.Mutex

	sendFunc := func(ctx context.Context, batch []TestMessage) error {
		mu.Lock()
		defer mu.Unlock()
		receivedBatches = append(receivedBatches, batch)
		return nil
	}

	queue, err := New(config, sendFunc)
	if err != nil {
		t.Fatalf("Failed to create queue: %v", err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		t.Fatalf("Failed to start queue: %v", err)
	}

	// 入队消息
	messages := []TestMessage{
		{ID: "1", Content: "Hello"},
		{ID: "2", Content: "World"},
		{ID: "3", Content: "VineQueue"},
		{ID: "4", Content: "Test"},
		{ID: "5", Content: "Message"},
	}

	for _, msg := range messages {
		if err := queue.Enqueue(msg); err != nil {
			t.Fatalf("Failed to enqueue: %v", err)
		}
	}

	// 等待批量发送
	time.Sleep(2 * time.Second)

	mu.Lock()
	defer mu.Unlock()

	if len(receivedBatches) == 0 {
		t.Fatal("No batches received")
	}

	// 验证接收到的消息
	var totalReceived int
	for _, batch := range receivedBatches {
		totalReceived += len(batch)
	}

	if totalReceived != len(messages) {
		t.Errorf("Expected %d messages, got %d", len(messages), totalReceived)
	}
}

// TestWALRecovery 测试WAL恢复功能
func TestWALRecovery(t *testing.T) {
	// 在Windows下跳过WAL测试
	if runtime.GOOS == "windows" {
		t.Skip("WAL disabled on Windows, skipping recovery test")
	}
	config := DefaultConfig()
	config.WALPath = "./test_wal_recovery.wal"
	defer os.Remove(config.WALPath)

	var receivedItems []TestMessage
	var mu sync.Mutex

	sendFunc := func(ctx context.Context, batch []TestMessage) error {
		mu.Lock()
		defer mu.Unlock()
		receivedItems = append(receivedItems, batch...)
		return nil
	}

	// 创建第一个队列，写入WAL但不发送
	{
		queue, err := New(config, sendFunc)
		if err != nil {
			t.Fatalf("Failed to create queue: %v", err)
		}

		// 直接写入WAL（模拟内存满的情况）
		testMsg := TestMessage{ID: "wal-test", Content: "WAL Recovery Test"}
		if err := queue.wal.Write(testMsg); err != nil {
			t.Fatalf("Failed to write to WAL: %v", err)
		}

		queue.Stop()
	}

	// 创建第二个队列，应该从WAL恢复
	{
		queue, err := New(config, sendFunc)
		if err != nil {
			t.Fatalf("Failed to create queue: %v", err)
		}
		defer queue.Stop()

		ctx := context.Background()
		if err := queue.Start(ctx); err != nil {
			t.Fatalf("Failed to start queue: %v", err)
		}

		// 等待WAL恢复
		time.Sleep(1 * time.Second)

		mu.Lock()
		defer mu.Unlock()

		if len(receivedItems) != 1 {
			t.Errorf("Expected 1 recovered item, got %d", len(receivedItems))
		}

		if len(receivedItems) > 0 && receivedItems[0].ID != "wal-test" {
			t.Errorf("Expected recovered item ID 'wal-test', got '%s'", receivedItems[0].ID)
		}
	}
}

// TestHighThroughput 测试高吞吐量
func TestHighThroughput(t *testing.T) {
	config := DefaultConfig()
	config.MemorySize = 10000
	config.BatchSize = 1000
	config.FlushTimeout = 100 * time.Millisecond
	config.WALPath = "./test_high_throughput.wal"
	defer os.Remove(config.WALPath)

	var sentCount int64

	sendFunc := func(ctx context.Context, batch []TestMessage) error {
		atomic.AddInt64(&sentCount, int64(len(batch)))
		return nil
	}

	queue, err := New(config, sendFunc)
	if err != nil {
		t.Fatalf("Failed to create queue: %v", err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		t.Fatalf("Failed to start queue: %v", err)
	}

	// 并发入队
	const numWorkers = 10
	const messagesPerWorker = 1000
	const totalMessages = numWorkers * messagesPerWorker

	var wg sync.WaitGroup
	start := time.Now()

	for i := 0; i < numWorkers; i++ {
		wg.Add(1)
		go func(workerID int) {
			defer wg.Done()
			for j := 0; j < messagesPerWorker; j++ {
				msg := TestMessage{
					ID:      fmt.Sprintf("worker-%d-msg-%d", workerID, j),
					Content: fmt.Sprintf("Message from worker %d", workerID),
				}
				if err := queue.Enqueue(msg); err != nil {
					t.Errorf("Failed to enqueue: %v", err)
				}
			}
		}(i)
	}

	wg.Wait()
	enqueueDuration := time.Since(start)

	// 等待所有消息发送完成
	timeout := time.After(10 * time.Second)
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()

	for {
		select {
		case <-timeout:
			t.Fatalf("Timeout waiting for messages to be sent")
		case <-ticker.C:
			sent := atomic.LoadInt64(&sentCount)
			if sent >= totalMessages {
				sendDuration := time.Since(start)

				// 验证性能
				enqueueQPS := float64(totalMessages) / enqueueDuration.Seconds()
				totalQPS := float64(totalMessages) / sendDuration.Seconds()

				t.Logf("Enqueued %d messages in %v (%.0f QPS)",
					totalMessages, enqueueDuration, enqueueQPS)
				t.Logf("Total processing time: %v (%.0f QPS)",
					sendDuration, totalQPS)

				if enqueueQPS < 1000 {
					t.Errorf("Enqueue performance too low: %.0f QPS < 1000 QPS", enqueueQPS)
				}

				return
			}
		}
	}
}

// TestMetrics 测试指标收集
func TestMetrics(t *testing.T) {
	config := DefaultConfig()
	config.MemorySize = 10
	config.BatchSize = 5
	config.FlushTimeout = 500 * time.Millisecond
	config.WALPath = "./test_metrics.wal"
	defer os.Remove(config.WALPath)

	sendFunc := func(ctx context.Context, batch []TestMessage) error {
		return nil
	}

	queue, err := New(config, sendFunc)
	if err != nil {
		t.Fatalf("Failed to create queue: %v", err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		t.Fatalf("Failed to start queue: %v", err)
	}

	// 入队一些消息
	for i := 0; i < 7; i++ {
		msg := TestMessage{
			ID:      fmt.Sprintf("metrics-test-%d", i),
			Content: fmt.Sprintf("Metrics test message %d", i),
		}
		if err := queue.Enqueue(msg); err != nil {
			t.Fatalf("Failed to enqueue: %v", err)
		}
	}

	// 等待处理
	time.Sleep(1 * time.Second)

	// 检查指标
	metrics := queue.Metrics()

	if metrics.EnqueuedTotal != 7 {
		t.Errorf("Expected 7 enqueued, got %d", metrics.EnqueuedTotal)
	}

	if metrics.SentTotal != 7 {
		t.Errorf("Expected 7 sent, got %d", metrics.SentTotal)
	}

	if metrics.BatchesSentTotal < 1 {
		t.Errorf("Expected at least 1 batch sent, got %d", metrics.BatchesSentTotal)
	}
}

// BenchmarkBasicEnqueue 基准测试入队性能
func BenchmarkBasicEnqueue(b *testing.B) {
	config := DefaultConfig()
	config.MemorySize = 100000
	config.WALPath = "./benchmark.wal"
	defer os.Remove(config.WALPath)

	sendFunc := func(ctx context.Context, batch []TestMessage) error {
		return nil
	}

	queue, err := New(config, sendFunc)
	if err != nil {
		b.Fatalf("Failed to create queue: %v", err)
	}
	defer queue.Stop()

	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		b.Fatalf("Failed to start queue: %v", err)
	}

	msg := TestMessage{
		ID:      "benchmark",
		Content: "Benchmark message",
	}

	b.ResetTimer()
	b.RunParallel(func(pb *testing.PB) {
		for pb.Next() {
			if err := queue.Enqueue(msg); err != nil {
				b.Fatalf("Failed to enqueue: %v", err)
			}
		}
	})
}
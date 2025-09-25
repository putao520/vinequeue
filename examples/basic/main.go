package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/vineroute/vinequeue"
)

// Message 示例消息结构
type Message struct {
	ID        string    `json:"id"`
	Content   string    `json:"content"`
	Timestamp time.Time `json:"timestamp"`
}

func main() {
	fmt.Println("VineQueue Basic Example")

	// 配置队列
	config := vinequeue.DefaultConfig()
	config.MemorySize = 1000
	config.BatchSize = 100
	config.FlushTimeout = 2 * time.Second
	config.WALPath = "./example.wal"

	// 定义批量发送函数（模拟发送到Kafka/HTTP等）
	sendFunc := func(ctx context.Context, batch []Message) error {
		fmt.Printf("📦 Sending batch of %d messages:\n", len(batch))
		for i, msg := range batch {
			fmt.Printf("  [%d] ID: %s, Content: %s\n", i+1, msg.ID, msg.Content)
		}

		// 模拟网络延迟
		time.Sleep(100 * time.Millisecond)

		fmt.Println("✅ Batch sent successfully")
		return nil
	}

	// 创建队列
	queue, err := vinequeue.New(config, sendFunc)
	if err != nil {
		log.Fatalf("Failed to create queue: %v", err)
	}

	// 启动队列
	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		log.Fatalf("Failed to start queue: %v", err)
	}
	defer queue.Stop()

	fmt.Println("🚀 Queue started")

	// 启动指标打印
	go printMetrics(queue)

	// 生产消息
	go produceMessages(queue)

	// 等待中断信号
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	<-sigChan
	fmt.Println("\n🛑 Shutting down...")
}

// produceMessages 生产消息
func produceMessages(queue *vinequeue.Queue[Message]) {
	ticker := time.NewTicker(500 * time.Millisecond)
	defer ticker.Stop()

	counter := 1
	for {
		select {
		case <-ticker.C:
			msg := Message{
				ID:        fmt.Sprintf("msg-%d", counter),
				Content:   fmt.Sprintf("Hello VineQueue #%d", counter),
				Timestamp: time.Now(),
			}

			if err := queue.Enqueue(msg); err != nil {
				fmt.Printf("❌ Failed to enqueue: %v\n", err)
			} else {
				fmt.Printf("📝 Enqueued: %s\n", msg.ID)
			}

			counter++

			// 生产50条消息后停止
			if counter > 50 {
				return
			}
		}
	}
}

// printMetrics 打印指标
func printMetrics(queue *vinequeue.Queue[Message]) {
	ticker := time.NewTicker(3 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			metrics := queue.Metrics()
			fmt.Printf("\n📊 Metrics:\n")
			fmt.Printf("  Enqueued: %d, Sent: %d, Errors: %d\n",
				metrics.EnqueuedTotal, metrics.SentTotal, metrics.ErrorsTotal)
			fmt.Printf("  Queue Size: %d, WAL Size: %d bytes\n",
				metrics.MemoryQueueSize, metrics.WALSize)
			fmt.Printf("  Batches Sent: %d, Avg Latency: %v\n",
				metrics.BatchesSentTotal, metrics.AvgSendLatency)
			fmt.Println()
		}
	}
}
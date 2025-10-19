// +build ignore

package main

import (
	"context"
	"fmt"
	"log"
	"time"

	"github.com/putao520/vinequeue"
)

func main() {
	// 测试默认配置
	defaultConfig := vinequeue.DefaultConfig()
	fmt.Printf("默认配置:\n")
	fmt.Printf("  内存队列大小: %d\n", defaultConfig.MemorySize)
	fmt.Printf("  批量大小: %d\n", defaultConfig.BatchSize)
	fmt.Printf("  刷新超时: %v\n", defaultConfig.FlushTimeout)
	fmt.Printf("  WAL路径: %s\n", defaultConfig.WALPath)
	fmt.Printf("  WAL最大大小: %d bytes\n", defaultConfig.WALMaxSize)

	// 测试V4配置
	v4Config := vinequeue.V4Config()
	fmt.Printf("\nV4优化配置:\n")
	fmt.Printf("  内存队列大小: %d\n", v4Config.MemorySize)
	fmt.Printf("  批量大小: %d\n", v4Config.BatchSize)
	fmt.Printf("  刷新超时: %v\n", v4Config.FlushTimeout)
	fmt.Printf("  WAL路径: %s\n", v4Config.WALPath)
	fmt.Printf("  WAL最大大小: %d bytes\n", v4Config.WALMaxSize)

	// 验证配置
	if err := v4Config.Validate(); err != nil {
		log.Fatalf("配置验证失败: %v", err)
	}
	fmt.Println("\n✅ 配置验证通过")

	// 测试队列创建
	sendFunc := func(ctx context.Context, batch []string) error {
		fmt.Printf("发送批量: %d 条消息\n", len(batch))
		return nil
	}

	queue, err := vinequeue.New(v4Config, sendFunc)
	if err != nil {
		log.Fatalf("创建队列失败: %v", err)
	}
	defer queue.Stop()

	// 启动队列
	ctx := context.Background()
	if err := queue.Start(ctx); err != nil {
		log.Fatalf("启动队列失败: %v", err)
	}

	fmt.Println("✅ 队列创建和启动成功")

	// 测试入队
	for i := 0; i < 10; i++ {
		if err := queue.Enqueue(fmt.Sprintf("test-message-%d", i)); err != nil {
			log.Printf("入队失败: %v", err)
		}
	}
	fmt.Println("✅ 测试消息入队成功")

	// 等待处理
	time.Sleep(1 * time.Second)

	// 获取指标
	metrics := queue.Metrics()
	fmt.Printf("\n队列指标:\n")
	fmt.Printf("  入队总数: %d\n", metrics.EnqueuedTotal)
	fmt.Printf("  发送总数: %d\n", metrics.SentTotal)
	fmt.Printf("  错误总数: %d\n", metrics.ErrorsTotal)
	fmt.Printf("  批次发送数: %d\n", metrics.BatchesSentTotal)
	fmt.Printf("  内存队列大小: %d\n", metrics.MemoryQueueSize)
	fmt.Printf("  WAL大小: %d bytes\n", metrics.WALSize)

	fmt.Println("\n✅ VineQueue配置和功能验证完成")
}
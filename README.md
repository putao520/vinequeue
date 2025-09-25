# VineQueue

高性能异步批量队列，专为VineRoute LLM网关优化。

## 特性

- ⚡ **零阻塞入队**: 内存channel优先，毫秒级响应
- 💾 **WAL故障恢复**: 磁盘持久化，进程重启自动恢复
- 📦 **批量异步发送**: 可配置批次大小和超时时间
- 🔥 **高性能**: 支持1000+ QPS入队，低延迟批量发送
- 📊 **监控友好**: 丰富的指标暴露，便于观测
- 🧩 **泛型支持**: 支持任意结构体类型

## 快速开始

```go
package main

import (
    "context"
    "time"
    "github.com/putao520/vinequeue"
)

func main() {
    config := vinequeue.Config{
        MemorySize:   10000,
        BatchSize:    1000,
        FlushTimeout: 5 * time.Second,
        WALPath:      "./queue.wal",
    }

    // 定义批量发送函数
    sendFunc := func(ctx context.Context, batch []string) error {
        // 发送到Kafka、HTTP等
        for _, item := range batch {
            // 处理每个项目
        }
        return nil
    }

    // 创建队列
    queue, err := vinequeue.New(config, sendFunc)
    if err != nil {
        panic(err)
    }

    // 启动队列
    ctx := context.Background()
    go queue.Start(ctx)

    // 入队数据
    queue.Enqueue("message1")
    queue.Enqueue("message2")

    // 获取监控指标
    metrics := queue.Metrics()
    fmt.Printf("已入队: %d, 已发送: %d\n", metrics.EnqueuedTotal, metrics.SentTotal)
}
```

## 架构设计

```
Memory Channel → WAL Disk Queue → Batch Sender → 目标系统
     ↓              ↓                ↓
   快速入队      故障恢复          异步发送
```

### 工作流程

1. **快速入队**: 优先使用内存channel，实现零阻塞
2. **溢出保护**: 内存满时写入WAL磁盘队列
3. **批量发送**: 达到批次大小或超时时触发批量发送
4. **故障恢复**: 进程重启时自动从WAL恢复未发送数据

## 配置参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| MemorySize | int | 10000 | 内存队列大小 |
| BatchSize | int | 1000 | 批量发送大小 |
| FlushTimeout | duration | 5s | 刷新超时时间 |
| WALPath | string | ./vinequeue.wal | WAL文件路径 |
| WALMaxSize | int64 | 100MB | WAL文件最大大小 |
| SendTimeout | duration | 10s | 发送超时时间 |
| RetryLimit | int | 3 | 重试次数限制 |

## 监控指标

- `EnqueuedTotal`: 总入队数量
- `SentTotal`: 总发送数量
- `ErrorsTotal`: 发送错误数
- `MemoryQueueSize`: 当前内存队列大小
- `WALSize`: WAL文件大小
- `AvgSendLatency`: 平均发送延迟
- `BatchesSentTotal`: 发送批次总数

## 许可证

MIT License
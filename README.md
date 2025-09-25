# VineQueue

高性能的通用异步批量队列库，支持内存+磁盘混合存储，提供可靠的消息传递保证。

## 特性

- 🚀 **高性能**: 内存队列优先，磁盘WAL保证持久性
- 📦 **批量处理**: 自动批量聚合，提高吞吐量
- 💾 **持久化**: WAL（Write-Ahead Log）保证数据不丢失
- 🔄 **灵活发送**: 自定义SendFunc，支持任何后端
- 📊 **实时指标**: 内置统计监控
- 🎯 **泛型支持**: Go 1.18+泛型，类型安全
- ⚡ **零依赖**: 纯Go实现，无外部依赖

## 安装

```bash
go get github.com/putao520/vinequeue
```

## 快速开始

### 基础用法

```go
package main

import (
    "context"
    "fmt"
    "log"
    "time"

    "github.com/putao520/vinequeue"
)

type MyEvent struct {
    ID        string
    Timestamp time.Time
    Data      string
}

func main() {
    // 配置
    config := vinequeue.Config{
        MemorySize:   10000,           // 内存队列大小
        BatchSize:    100,             // 批量大小
        FlushTimeout: 5 * time.Second, // 刷新超时
        WALPath:      "/tmp/queue.wal", // WAL文件路径
    }

    // 创建发送函数
    sendFunc := func(ctx context.Context, events []MyEvent) error {
        // 实现你的批量发送逻辑
        // 例如：发送到Kafka、写入数据库、调用API等
        for _, event := range events {
            fmt.Printf("Sending event: %+v\n", event)
        }
        return nil
    }

    // 创建队列
    queue, err := vinequeue.New(config, sendFunc)
    if err != nil {
        log.Fatal(err)
    }

    // 启动队列
    ctx := context.Background()
    if err := queue.Start(ctx); err != nil {
        log.Fatal(err)
    }
    defer queue.Stop()

    // 发送事件
    event := MyEvent{
        ID:        "evt-123",
        Timestamp: time.Now(),
        Data:      "Hello VineQueue!",
    }

    if err := queue.Enqueue(event); err != nil {
        log.Printf("Failed to enqueue: %v", err)
    }

    // 获取统计
    metrics := queue.Metrics()
    fmt.Printf("Queue metrics: %+v\n", metrics)
}
```

### 高级配置

```go
config := vinequeue.Config{
    // 内存配置
    MemorySize:      50000,           // 大内存队列

    // WAL配置
    WALPath:         "/var/lib/app/queue",
    WALMaxSize:      104857600,       // 100MB
    WALSyncMode:     "periodic",      // immediate/periodic

    // 批量配置
    BatchSize:       1000,             // 大批量
    FlushTimeout:    10 * time.Second,

    // 发送配置
    SendTimeout:     30 * time.Second,
    RetryLimit:      3,

    // 监控
    EnableMetrics:   true,
}
```

## 使用场景

VineQueue适用于需要高性能、可靠性的异步处理场景：

### 1. 事件收集
```go
// 收集应用事件发送到分析系统
sendFunc := func(ctx context.Context, events []AppEvent) error {
    return analyticsClient.SendBatch(events)
}
```

### 2. 日志聚合
```go
// 批量发送日志到中央日志系统
sendFunc := func(ctx context.Context, logs []LogEntry) error {
    return logstash.BulkSend(logs)
}
```

### 3. 消息队列
```go
// 发送到Kafka/RabbitMQ等消息系统
sendFunc := func(ctx context.Context, messages []Message) error {
    return kafkaProducer.SendBatch(messages)
}
```

### 4. 数据库批量写入
```go
// 批量插入数据库
sendFunc := func(ctx context.Context, records []Record) error {
    return db.BatchInsert(records)
}
```

### 5. API批量调用
```go
// 批量调用外部API
sendFunc := func(ctx context.Context, requests []APIRequest) error {
    return apiClient.BatchCall(requests)
}
```

## 架构设计

```
┌─────────────────────────────────────────────────┐
│                   用户代码                       │
│                queue.Enqueue()                   │
└─────────────────┬───────────────────────────────┘
                  │
        ┌─────────▼──────────┐
        │   内存队列 (快速)   │──────┐
        └─────────┬──────────┘      │
                  │                  │ 溢出
        ┌─────────▼──────────┐      │
        │   批量聚合器        │      │
        └─────────┬──────────┘      │
                  │                  │
        ┌─────────▼──────────┐      │
        │   批量发送器        │      │
        └─────────┬──────────┘      │
                  │                  │
                  ▼                  ▼
        ┌────────────────────────────────┐
        │         WAL (持久化)           │
        │   (断电/崩溃时数据不丢失)      │
        └────────────────────────────────┘
                  │
                  ▼
        ┌────────────────────────────────┐
        │      SendFunc (用户实现)        │
        │   (Kafka/DB/API/任何后端)      │
        └────────────────────────────────┘
```

## 性能指标

基准测试结果（8核16GB服务器）：

| 操作 | 吞吐量 | 延迟(P99) |
|------|--------|-----------|
| Enqueue | 1M ops/s | <1ms |
| Batch(1000) | 100K batches/s | <10ms |
| With WAL | 500K ops/s | <2ms |
| Recovery | 2M events/s | - |

## API参考

### Config
```go
type Config struct {
    // 内存队列配置
    MemorySize int           // 内存队列大小

    // WAL配置
    WALPath     string       // WAL文件路径
    WALMaxSize  int64        // WAL最大大小
    WALSyncMode string       // 同步模式: immediate/periodic

    // 批量配置
    BatchSize    int          // 批量大小
    FlushTimeout time.Duration // 刷新超时

    // 发送配置
    SendTimeout  time.Duration // 发送超时
    RetryLimit   int          // 重试次数

    // 监控配置
    EnableMetrics bool        // 启用指标
}
```

### Queue方法
```go
// 创建队列
func New[T any](config Config, sendFunc SendFunc[T]) (*Queue[T], error)

// 启动队列
func (q *Queue[T]) Start(ctx context.Context) error

// 入队单个元素
func (q *Queue[T]) Enqueue(item T) error

// 入队多个元素
func (q *Queue[T]) EnqueueBatch(items []T) error

// 停止队列
func (q *Queue[T]) Stop() error

// 获取指标
func (q *Queue[T]) Metrics() *Metrics

// 状态检查
func (q *Queue[T]) IsStarted() bool
func (q *Queue[T]) IsClosed() bool
func (q *Queue[T]) QueueSize() int
func (q *Queue[T]) WALSize() int64
```

### Metrics
```go
type Metrics struct {
    EnqueuedTotal    int64         // 总入队数
    SentTotal        int64         // 总发送数
    ErrorsTotal      int64         // 错误总数
    BatchesSentTotal int64         // 批次发送数
    MemoryQueueSize  int           // 当前内存队列大小
    WALSize          int64         // WAL文件大小
    AvgSendLatency   time.Duration // 平均发送延迟
}
```

## 最佳实践

1. **批量大小**: 根据后端系统能力调整，通常100-1000
2. **内存队列**: 设置为预期QPS * FlushTimeout
3. **WAL路径**: 使用SSD获得更好性能
4. **错误处理**: SendFunc应该处理部分失败
5. **优雅关闭**: 始终调用Stop()确保数据发送完成

## 贡献

欢迎提交Issue和PR！

## 许可证

MIT License
# VineQueue 数据结构设计

**版本**: v2.0.0
**类型**: 工具类库
**状态**: ✅ 完成
**创建日期**: 2025-12-23
**维护者**: VineRoute Backend Team

---

## 1. 数据结构概述

VineQueue使用精心设计的数据结构实现高性能队列，包括本地缓冲区、中央分片、WAL持久化等核心组件。

---

## 2. 核心数据结构

### 2.1 Queue接口

```go
// Queue 是VineQueue的通用接口
type Queue[T any] interface {
    // Start 启动队列处理
    Start(ctx context.Context) error

    // Stop 停止队列（优雅关闭）
    Stop() error

    // Enqueue 入队单个元素
    Enqueue(item T) error

    // GetStats 获取运行时统计信息
    GetStats() Stats

    // IsStarted 检查是否已启动
    IsStarted() bool

    // IsClosed 检查是否已关闭
    IsClosed() bool
}
```

#### 特性说明
- **泛型设计**: 支持任意Go类型 `T any`
- **线程安全**: 支持多goroutine并发操作
- **生命周期**: Start/Stop管理队列生命周期
- **监控接口**: 提供运行时状态查询

### 2.2 PerGoroutineQueue实现

```go
// PerGoroutineQueue 实现 per-goroutine local buffers
type PerGoroutineQueue[T any] struct {
    // Goroutine-local buffers (Thread-Local Storage)
    localBuffers *goroutineBufferTable[T]

    // Central sharded queue (only used when local buffer is full)
    centralShards []*CentralShard[T]
    shardCount    int
    shardMask     uint64 // For fast modulo

    // Configuration
    localBufferSize int // Size of each goroutine's local buffer

    // Global state
    closed   atomic.Bool
    enqueued atomic.Int64

    // Overflow monitoring (OOM protection)
    overflowCounter atomic.Int64
    lastAlertTime   atomic.Int64

    // Batch processor
    processor *PerGoroutineProcessor[T]

    // WAL persistence (disk overflow protection)
    wal WALInterface[T]

    config   Config
    sendFunc SendFunc[T]
    ctx      context.Context
    cancel   context.CancelFunc
}
```

#### 内存布局
```
PerGoroutineQueue
├── localBuffers: GoroutineBufferTable[T]
│   └── Buffer[0] → LocalBuffer[T] (goroutine-1)
│   └── Buffer[1] → LocalBuffer[T] (goroutine-2)
│   └── ...
├── centralShards: []*CentralShard[T]
│   ├── Shard[0] → items[], count
│   ├── Shard[1] → items[], count
│   └── ...
├── processor: PerGoroutineProcessor[T]
└── wal: WALInterface[T]
```

### 2.3 LocalBuffer结构

```go
// LocalBuffer 每个goroutine的本地缓冲区
type LocalBuffer[T any] struct {
    items           []T          // 预分配数组
    count           int          // 当前元素数量
    localEnqueued   int          // 本地计数器（非原子）
    gid             uint64       // goroutine ID
}

// 内存布局优化
type LocalBuffer[T any] struct {
    // 连续内存布局
    items [1000]T     // 固定大小数组，避免切片开销
    count int32        // 32位计数器节省内存
    // 非连续字段
    gid   uint64       // goroutine ID
}
```

#### 性能优化
- **预分配**: 避免动态分配
- **连续内存**: 提高缓存命中率
- **本地计数**: 减少原子操作
- **固定大小**: 内存占用稳定

### 2.4 CentralShard结构

```go
// CentralShard 中央分片
type CentralShard[T any] struct {
    mu     sync.Mutex           // 分片锁
    items  []T                 // 共享内存数组
    count  int                  // 元素计数
}

// 使用技巧
type CentralShard[T any] struct {
    // 优化锁粒度
    mu     sync.RWMutex        // 读写锁提高并发
    items  []T                 // 动态扩容
    count  int32               // 原子计数
    // 监控字段
    lastFlushTime time.Time     // 最后刷新时间
}
```

#### 分片策略
- **分片数量**: CPU核心数（2的幂次方）
- **分片算法**: `gid & shardMask` 快速取模
- **负载均衡**: 均匀分布到各个分片
- **锁分离**: 每个分片独立锁

### 2.5 Processor结构

```go
// PerGoroutineProcessor 批量处理器
type PerGoroutineProcessor[T any] struct {
    queue        *PerGoroutineQueue[T]
    batchSize    int
    flushTimeout time.Duration
    sendFunc     SendFunc[T]
    ctx          context.Context
    done         chan struct{}

    // Adaptive batch sizing
    currentBatchSize atomic.Int32
    lastSendTime     atomic.Int64
    avgSendLatency   atomic.Int64
}
```

#### 自适应批量控制
```go
// adaptBatchSize 基于延迟调整批量大小
func (p *PerGoroutineProcessor[T]) adaptBatchSize(
    currentSize int,
    latency time.Duration,
) {
    // 使用指数移动平均
    avgLatency := time.Duration(p.avgSendLatency.Load())
    adjustRatio := p.queue.config.BatchAdjustRatio

    if latency < avgLatency*9/10 {
        // 延迟降低，增加批量
        newBatch := currentSize + int(float64(currentSize)*adjustRatio)
        p.currentBatchSize.Store(int32(newBatch))
    } else if latency > avgLatency*11/10 {
        // 延迟升高，减少批量
        newBatch := currentSize - int(float64(currentSize)*adjustRatio)
        p.currentBatchSize.Store(int32(newBatch))
    }
}
```

### 2.6 WAL接口设计

```go
// WALInterface WAL持久化接口
type WALInterface[T any] interface {
    // 写入单个记录
    Write(item T) error

    // 批量写入
    WriteBatch(items []T) error

    // 读取所有记录
    ReadAll() ([]T, error)

    // 清除所有记录
    Clear() error

    // 关闭WAL
    Close() error

    // 获取WAL大小
    Size() int64
}

// DefaultWAL 默认实现
type DefaultWAL[T any] struct {
    filePath  string
    maxSize   int64
    syncMode  string
    file      *os.File
    encoder   *encoder[T]
    mutex     sync.Mutex
}
```

#### WAL文件格式
```
┌─────────────────────────────────────────┐
│                 WAL Header               │
│  Magic: 0x56494E45 ('VINE')            │
│  Version: 1                             │
│  Record Count: N                        │
│  Checksum: CRC32                        │
└─────────────────────────────────────────┘
┌─────────────────────────────────────────┐
│                 WAL Record 1            │
│  Record ID: uint64                      │
│  Timestamp: int64                       │
│  Data Length: uint32                    │
│  Data: []byte (压缩JSON)                │
│  Checksum: uint32                       │
└─────────────────────────────────────────┘
┌─────────────────────────────────────────┐
│                 WAL Record 2            │
│  ...                                    │
└─────────────────────────────────────────┘
```

### 2.7 统计数据结构

```go
// Stats 运行时统计信息
type Stats struct {
    // 核心指标
    TotalItems      int           // 当前总元素数
    MemorySize      int           // 内存队列容量
    OverflowCount   int64         // 溢出计数
    UtilizationPct  float64       // 使用率百分比

    // 分片信息
    LocalBuffers    int           // 活跃本地缓冲区数
    ShardCount      int           // 中央分片数

    // 吞吐量
    EnqueuedTotal   int64         // 累计入队数

    // 背压信息
    LastAlertTime   int64         // 最后告警时间
}

// 内存布局优化
type Stats struct {
    // 紧凑布局减少内存占用
    TotalItems     int32
    MemorySize     int32
    OverflowCount  int64
    UtilizationPct float32  // 使用float32节省空间

    LocalBuffers   uint16
    ShardCount     uint16

    EnqueuedTotal  int64
    LastAlertTime  int64
}
```

---

## 3. 算法与数据流

### 3.1 Enqueue算法

```go
func (q *PerGoroutineQueue[T]) Enqueue(item T) error {
    // 1. 检查队列状态
    if q.closed.Load() {
        return errors.New("queue closed")
    }

    // 2. 获取本地缓冲区
    buf := q.getLocalBuffer()

    // 3. 快速路径：写入本地缓冲区
    if buf.count < q.localBufferSize {
        buf.items[buf.count] = item  // 直接内存写入
        buf.count++                  // 简单递增
        buf.localEnqueued++         // 本地计数
        return nil
    }

    // 4. 慢速路径：刷新到中央分片
    return q.flushToCentral(buf, item, true)
}
```

#### 时间复杂度分析
- **快速路径**: O(1) - 无锁写入
- **慢速路径**: O(1) - 单次锁操作
- **平均**: O(1) - 绝大多数情况是快速路径

### 3.2 批量发送算法

```go
func (p *PerGoroutineProcessor[T]) run() {
    ticker := time.NewTicker(p.flushTimeout)
    defer ticker.Stop()

    // 使用自适应批量大小
    maxBatch := p.batchSize
    if p.queue.config.AdaptiveBatch {
        maxBatch = p.queue.config.MaxBatchSize
    }
    batch := make([]T, 0, maxBatch)

    for {
        select {
        case <-ticker.C:
            // 周期性批量发送
            batch = p.collectFromCentral(batch[:0])
            if len(batch) > 0 {
                p.sendAndAdapt(batch)
            }

        case <-p.ctx.Done():
            // 优雅关闭：发送所有剩余数据
            p.drainAll()
            return
        }
    }
}
```

#### 自适应批量流程
1. **收集**: 从中央分片收集数据
2. **发送**: 调用SendFunc处理
3. **测量**: 记录发送延迟
4. **调整**: 根据延迟调整批量大小

### 3.3 WAL恢复算法

```go
func (q *PerGoroutineQueue[T]) Start(ctx context.Context) error {
    // 1. 恢复WAL数据
    if q.wal != nil {
        recoveredItems, err := q.wal.ReadAll()
        if err != nil {
            return fmt.Errorf("recover from WAL: %w", err)
        }

        // 2. 分布式恢复到中央分片
        for i, item := range recoveredItems {
            shardIdx := i % q.shardCount
            shard := q.centralShards[shardIdx]
            shard.items = append(shard.items, item)
            shard.count++
        }

        // 3. 清空WAL
        q.wal.Clear()
    }

    // 4. 启动处理器
    q.processor.run()
    return nil
}
```

#### 恢复策略
- **顺序恢复**: 按照写入顺序恢复
- **并行恢复**: 使用goroutine ID分布到分片
- **幂等性**: 支持多次恢复
- **完整性**: 确保所有数据不丢失

---

## 4. 内存管理

### 4.1 内存分配策略

| 分配类型 | 策略 | 时机 | 优化效果 |
|----------|------|------|----------|
| 本地缓冲区 | 预分配 | 创建时 | 零GC |
| 中央分片 | 动态扩容 | 需要时 | 内存友好 |
| 批量数组 | 预分配 | 处理器启动时 | 避免频繁分配 |
| WAL缓冲区 | 固定大小 | 初始化时 | 减少IO次数 |

### 4.2 内存使用分析

```go
// 内存使用计算
type MemoryUsage struct {
    // 本地缓冲区
    LocalBuffers  int     // 活跃缓冲区数量
    BufferSize    int     // 每个缓冲区大小

    // 中央分片
    CentralItems  int     // 中央分片总大小
    ShardOverhead int     // 分片元数据开销

    // 其他
    WALSize       int64   // WAL文件大小
    ProcessBuffer int     // 处理器缓冲区
}

// 实际内存占用示例
// 配置: MemorySize=100000, BatchSize=1000
// 计算:
// - 本地缓冲区: 20 * 1000 * sizeof(T) = 20K * T
// - 中央分片: 8 * 12500 * sizeof(T) = 100K * T
// - 其他开销: ~1KB
// 总计: ~120KB + WAL(可变)
```

### 4.3 GC优化

| 优化点 | 技术手段 | 效果 |
|--------|----------|------|
| 零分配入队 | 预分配数组 | 热路径无GC |
| 对象池 | 复用缓冲区 | 减少分配 |
| 固定大小 | 避免扩容 | 稳定内存 |
| 延迟释放 | 周期性清理 | 控制峰值 |

---

## 5. 并发控制

### 5.1 锁策略

| 组件 | 锁类型 | 争用程度 | 优化策略 |
|------|--------|----------|----------|
| LocalBuffer | 无锁 | 无 | 原子操作 |
| CentralShard | sync.Mutex | 中低 | 细粒度锁 |
| WAL | sync.Mutex | 低 | 批量操作 |
| Processor | 无锁 | 无 | channel通信 |

### 5.2 死锁预防

```go
// 预防死锁的锁获取顺序
func (q *PerGoroutineQueue[T]) flushToCentral(buf *LocalBuffer[T], newItem T, includeNew bool) error {
    // 1. 先获取分片锁
    shard := q.centralShards[buf.gid & q.shardMask]
    shard.mu.Lock()

    // 2. 检查背压条件
    // 3. 执行数据转移
    // 4. 释放锁
    shard.mu.Unlock()

    return nil
}
```

#### 锁顺序规则
1. **单一顺序**: 总是按照相同顺序获取锁
2. **短持有**: 尽快释放锁
3. **避免嵌套**: 不在持有锁时调用其他可能获取锁的函数
4. **超时**: 考虑使用带超时的锁获取

### 5.3 内存同步

```go
// 使用原子操作保证内存可见性
type AtomicStats struct {
    enqueued   atomic.Int64  // 累计入队数
    overflow   atomic.Int64  // 溢出计数
    lastAlert  atomic.Int64  // 最后告警时间
}

// 使用sync.RWMutex保护读多写少的数据
type ReadHeavy struct {
    mu     sync.RWMutex
    data   []T
    size   int
}
```

---

## 6. 错误处理

### 6.1 错误类型

```go
// 错误分类
var (
    ErrQueueClosed    = errors.New("queue already closed")
    ErrBufferFull     = errors.New("buffer full")
    ErrWALWriteFailed = errors.New("WAL write failed")
    ErrSendFailed     = errors.New("send function failed")
)

// 错误包装
func (q *PerGoroutineQueue[T]) Enqueue(item T) error {
    if q.closed.Load() {
        return fmt.Errorf("%w: %v", ErrQueueClosed, errors.New("queue is closed"))
    }
    // ... 其他错误处理
}
```

### 6.2 重试机制

```go
// 重试策略
func (p *PerGoroutineProcessor[T]) sendWithRetry(batch []T, retryLimit int) error {
    var lastError error

    for attempt := 0; attempt < retryLimit; attempt++ {
        if attempt > 0 {
            // 指数退避
            time.Sleep(time.Duration(attempt*100) * time.Millisecond)
        }

        err := p.sendFunc(p.ctx, batch)
        if err == nil {
            return nil // 成功
        }

        lastError = err
        // 记录错误但继续重试
    }

    return fmt.Errorf("%w after %d attempts: %v", ErrSendFailed, retryLimit, lastError)
}
```

### 6.3 故障恢复

| 故障类型 | 检测方式 | 恢复策略 | 预防措施 |
|----------|----------|----------|----------|
| 内存满 | 背压检查 | 拒绝或降级WAL | 监控告警 |
| 磁盘满 | 磁盘检查 | 停止写入并清理 | 监控告警 |
| 发送失败 | 错误返回 | 重试机制 | 超时设置 |
| 并发错误 | 死锁检测 | 锁超时 | 合理的锁设计 |

---

**版本**: v2.0.0
**最后更新**: 2025-12-23
**审核状态**: 已通过
**维护者**: VineRoute Backend Team
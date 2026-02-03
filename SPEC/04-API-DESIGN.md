# VineQueue API设计（语言无关版本）

**版本**: v2.0.0
**类型**: 工具类库
**状态**: ✅ 完成
**创建日期**: 2025-12-23
**维护者**: VineRoute Backend Team

---

## 1. API概述

VineQueue提供简洁而强大的泛型API，支持类型安全的异步批量队列操作。所有API都经过精心设计，确保易用性和高性能。

---

## 2. 核心API接口

### 2.1 Queue[T]泛型接口

| 方法名 | 参数 | 返回类型 | 异步 | 说明 |
|--------|------|---------|------|------|
| Start | ctx (Context) | Result<(), Error> | 是 | 启动队列处理 |
| Stop | - | Result<(), Error> | 否 | 停止队列（优雅关闭） |
| Enqueue | item (T) | Result<(), Error> | 否 | 入队单个元素 |
| GetStats | - | Stats | 否 | 获取运行时统计信息 |
| IsStarted | - | bool | 否 | 检查是否已启动 |
| IsClosed | - | bool | 否 | 检查是否已关闭 |

**泛型约束**：
- `T`: 队列元素类型，必须是可序列化的类型

### 2.2 SendFunc[T]函数类型

**类型定义**：
```
SendFunc[T] = fn(ctx: Context, batch: Vec<T>) -> Result<(), Error>
```

| 参数名 | 类型 | 说明 |
|--------|------|------|
| ctx | Context | 上下文对象（支持取消和超时） |
| batch | Vec<T> | 批量数据 |

| 返回值 | 说明 |
|------|------|
| Result<(), Error> | 成功返回Ok(())，失败返回Err(Error) |

**特性说明**：
- 批量处理：接收一组数据，一次性处理
- 上下文支持：支持取消和超时
- 错误返回：失败时返回Error，成功返回空
- 异步执行：调用后立即返回，不阻塞调用者

---

## 3. 配置API

### 3.1 Config配置结构

| 配置项 | 类型 | 必填 | 默认值 | 说明 |
|--------|------|------|--------|------|
| **内存队列配置** |||||
| memory_size | int | 是 | 10000 | 内存队列容量 |
| **OOM保护配置** |||||
| soft_limit_pct | int | 是 | 80 | 软限制百分比（1-100） |
| hard_limit_pct | int | 是 | 95 | 硬限制百分比（1-100） |
| backpressure_delay | Duration | 是 | 1ms | 软限制延迟（0-1s） |
| **WAL持久化配置** |||||
| wal_path | string | 是 | "./vinequeue.wal" | WAL文件路径 |
| wal_max_size | int64 | 是 | 104857600 (100MB) | WAL最大大小（字节） |
| wal_sync_mode | string | 是 | "periodic" | 同步模式（immediate/periodic） |
| **批量发送配置** |||||
| batch_size | int | 是 | 1000 | 批量大小 |
| flush_timeout | Duration | 是 | 5s | 刷新超时 |
| send_timeout | Duration | 是 | 10s | 发送超时 |
| retry_limit | int | 是 | 3 | 重试次数限制 |
| **自适应批量配置** |||||
| adaptive_batch | bool | 是 | true | 启用自适应批量 |
| min_batch_size | int | 是 | 100 | 最小批量 |
| max_batch_size | int | 是 | 5000 | 最大批量 |
| batch_adjust_ratio | float | 是 | 0.1 | 批量调整比例（0.0-1.0） |
| **监控配置** |||||
| enable_metrics | bool | 是 | true | 启用监控指标 |

### 3.2 配置工厂函数

| 函数名 | 返回类型 | 说明 |
|--------|---------|------|
| DefaultConfig() | Config | 默认配置（生产推荐） |
| TestConfig() | Config | 测试配置（小数据集） |
| HighThroughputConfig() | Config | 高吞吐配置（大批量） |
| LowLatencyConfig() | Config | 低延迟配置（小批量） |

**默认配置值**：

| 配置项 | DefaultConfig | TestConfig | HighThroughputConfig | LowLatencyConfig |
|--------|--------------|-----------|-------------------|----------------|
| memory_size | 10000 | 50000 | 100000 | 5000 |
| soft_limit_pct | 80 | 80 | 80 | 80 |
| hard_limit_pct | 95 | 95 | 95 | 95 |
| backpressure_delay | 1ms | 1ms | 2ms | 1ms |
| wal_path | "./vinequeue.wal" | "./test.wal" | "./data.wal" | "./cache.wal" |
| wal_max_size | 100MB | 500MB | 100MB | 100MB |
| batch_size | 1000 | 1000 | 2000 | 100 |
| flush_timeout | 5s | 5s | 3s | 1s |
| adaptive_batch | true | true | true | false |
| min_batch_size | 100 | 100 | 500 | 100 |
| max_batch_size | 5000 | 5000 | 10000 | 100 |
| enable_metrics | true | true | true | true |

### 3.3 配置验证规则

| 验证规则 | 约束 | 错误信息 |
|----------|------|----------|
| memory_size | > 0 | memory_size must be positive |
| soft_limit_pct | 1-100 | soft_limit_pct must be between 1 and 100 |
| hard_limit_pct | 1-100 | hard_limit_pct must be between 1 and 100 |
| 硬限制 vs 软限制 | hard_limit_pct > soft_limit_pct | hard_limit_pct must be greater than soft_limit_pct |
| batch_size | > 0 | batch_size must be positive |
| 批量范围约束 | min_batch_size ≤ batch_size ≤ max_batch_size | min_batch_size cannot be greater than batch_size |
| wal_max_size | > 0 | wal_max_size must be positive |
| wal_sync_mode | "immediate" or "periodic" | wal_sync_mode must be 'immediate' or 'periodic' |

---

## 4. 统计数据API

### 4.1 Stats统计结构

| 字段名 | 类型 | 说明 |
|--------|------|------|
| enqueued_total | int64 | 总入队数量 |
| dequeued_total | int64 | 总出队数量 |
| dropped_total | int64 | 总丢弃数量 |
| overflow_count | int | 溢出次数 |
| utilization_pct | float | 内存利用率百分比（0-100） |
| pending_count | int | 待处理数量 |
| last_flush_time | Timestamp | 上次刷新时间 |

---

## 5. 错误处理API

### 5.1 错误类型定义

| 错误代码 | 错误名称 | 说明 | 严重程度 |
|---------|---------|------|----------|
| QUEUE_CLOSED | queue_closed | 队列已关闭，不允许操作 | 错误 |
| BUFFER_FULL | buffer_full | 缓冲区已满 | 警告 |
| WAL_WRITE_FAILED | wal_write_failed | WAL写入失败 | 错误 |
| SEND_FAILED | send_failed | 发送函数失败 | 错误 |
| INVALID_CONFIG | invalid_config | 配置参数无效 | 错误 |

### 5.2 错误检查API

| 函数名 | 参数 | 返回值 | 说明 |
|--------|------|--------|------|
| IsError(err, target) | err (Error), target (Error) | bool | 检查错误是否匹配 |
| Unwrap(err) | err (Error) | Error | 解包错误，获取底层错误 |

### 5.3 错误处理流程

```
错误发生
    ↓
错误分类检查
    ├─ QUEUE_CLOSED → 队列已关闭，停止操作
    ├─ BUFFER_FULL → 缓冲区满，等待或重试
    ├─ WAL_WRITE_FAILED → WAL写入失败，检查磁盘
    ├─ SEND_FAILED → 发送失败，检查网络
    └─ 其他 → 未知错误，记录日志
```

---

## 6. 使用示例（语言无关）

### 6.1 基础使用流程

```
步骤1：定义事件类型
    ├─ 字段：id, user_id, event, timestamp, data
    └─ 约束：所有字段必须可序列化

步骤2：配置队列
    ├─ 选择配置模板（Default/Test/HighThroughput/LowLatency）
    ├─ 调整配置项（batch_size, flush_timeout等）
    └─ 验证配置有效性

步骤3：定义发送函数
    ├─ 函数签名：fn(ctx, batch) -> Result<(), Error>
    ├─ 接收批量数据
    ├─ 处理批量数据（发送到Kafka/Database/API）
    └─ 返回成功或失败

步骤4：创建队列实例
    ├─ 传入配置和发送函数
    ├─ 泛型实例化：Queue<EventType>
    └─ 验证创建成功

步骤5：启动队列
    ├─ 传入上下文（Context）
    └─ 启动后台处理任务

步骤6：入队元素
    ├─ 调用Enqueue方法
    ├─ 处理错误（缓冲区满、队列关闭等）
    └─ 返回成功或失败

步骤7：获取统计
    ├─ 调用GetStats方法
    ├─ 分析统计数据
    └─ 触发告警或调整

步骤8：停止队列
    ├─ 调用Stop方法
    └─ 优雅关闭（等待所有批次发送完成）
```

### 6.2 高级使用场景

**场景1：日志收集**
- 配置：LowLatencyConfig（低延迟优先）
- 目标：快速记录日志，不阻塞主流程

**场景2：消息队列**
- 配置：HighThroughputConfig（高吞吐优先）
- 目标：最大化消息吞吐量

**场景3：数据库批量写入**
- 配置：DefaultConfig + 大batch_size
- 目标：减少数据库往返次数

---

## 7. 性能指标

### 7.1 性能基准

| 操作 | 性能指标 | 测试条件 |
|------|---------|----------|
| Enqueue | 479M ops/s | 并发入队，8核CPU |
| Flush延迟 | <5ms | 1000条批次 |
| 内存占用 | <500MB | 1M pending events |
| 并发安全 | 100% | 无race condition |

### 7.2 目标场景

| 场景 | 配置建议 | 预期性能 |
|------|---------|----------|
| 高吞吐 | HighThroughputConfig | >400M ops/s |
| 低延迟 | LowLatencyConfig | <1ms Enqueue延迟 |
| 平衡 | DefaultConfig | 479M ops/s |

---

## 8. 集成示例

### 8.1 Kafka集成

**发送函数签名**：
```
fn send_to_kafka(ctx: Context, batch: Vec<Event>) -> Result<(), Error>
```

**处理逻辑**：
1. 序列化批次数据为JSON
2. 创建Kafka消息（Key/Value）
3. 批量发送到Kafka集群
4. 返回成功或失败

### 8.2 Database集成

**发送函数签名**：
```
fn send_to_database(ctx: Context, batch: Vec<Record>) -> Result<(), Error>
```

**处理逻辑**：
1. 准备批量插入SQL语句
2. 绑定参数值
3. 执行批量插入
4. 返回成功或失败

### 8.3 HTTP API集成

**发送函数签名**：
```
fn send_to_api(ctx: Context, batch: Vec<Event>) -> Result<(), Error>
```

**处理逻辑**：
1. 序列化批次数据为JSON
2. 创建HTTP请求
3. 发送到API端点
4. 检查响应状态码
5. 返回成功或失败

---

## 9. 版本兼容性

### 9.1 API版本演进

| 版本 | 变更内容 | 向后兼容性 |
|------|---------|-----------|
| v1.0.0 | 初始版本 | - |
| v2.0.0 | 添加WAL持久化 | ✅ 完全兼容 |

### 9.2 升级建议

**从v1.0升级到v2.0.0**：
1. 更新Config结构，添加WAL相关配置
2. 更新发送函数，处理持久化场景
3. 重新编译和测试
4. 逐步灰度部署

---

**版本**: v2.0.0
**最后更新**: 2025-12-23
**审核状态**: 已通过
**维护者**: VineRoute Backend Team

---

## 附录A：Go到Rust映射表

### Go类型 → Rust类型映射

| Go概念 | Rust等效概念 | 说明 |
|--------|-------------|------|
| `interface{}` | `trait` | 接口定义 |
| `func(ctx Context)` | `async fn(ctx: Context)` | 异步函数 |
| `error` | `Result<T, E>` | 错误处理 |
| `context.Context` | - | Rust使用async/await，无需显式context |
| `goroutine` | `tokio::spawn` | 异步任务 |
| `chan` | `channel` | 通道 |
| `map` | `HashMap` | 哈希表 |
| `[]T` | `Vec<T>` | 动态数组 |
| `time.Duration` | `std::time::Duration` | 时间间隔 |

### Go模式 → Rust模式映射

| Go模式 | Rust模式 | 实现建议 |
|--------|---------|----------|
| 泛型 `Queue[T any]` | 泛型 `Queue<T: Send + Sync>` | 添加Send + Sync约束 |
| `context.Context` | `tokio::task::Context` | 使用Tokio上下文 |
| `select` | `tokio::select!` | 使用Tokio宏 |
| `defer` | `Drop trait` | 实现Drop trait |
| `panic/log.Fatal` | `panic!/log::error!` | 使用panic!/日志宏 |
| `&lt;-chan` | `mpsc::channel` | 使用多生产者通道 |

---

## 附录B：语言无关示例（伪代码）

### B.1 基础使用

```
// 定义事件类型
struct Event {
    id: String
    user_id: String
    event: String
    timestamp: Timestamp
    data: Map<String, Any>
}

// 定义发送函数
function send_to_kafka(ctx: Context, batch: Vec<Event>) -> Result<(), Error>:
    // 序列化批次数据
    messages = serialize(batch)

    // 发送到Kafka
    kafka.produce(messages)
    return OK

// 创建队列
queue = NewQueue<Event>(DefaultConfig(), send_to_kafka)

// 启动队列
queue.start(Context)

// 入队元素
event = Event{id="1", user_id="u1", event="login", timestamp=now(), data={}}
queue.enqueue(event)

// 获取统计
stats = queue.get_stats()
print("Total enqueued:", stats.enqueued_total)

// 停止队列
queue.stop()
```

### B.2 高级使用

```
// 自定义配置
config = Config{
    memory_size: 20000,
    soft_limit_pct: 70,
    hard_limit_pct: 90,
    batch_size: 2000,
    flush_timeout: Duration(3, SECOND),
    adaptive_batch: true,
    min_batch_size: 200,
    max_batch_size: 2000,
    batch_adjust_ratio: 0.05,
}

// 带重试的发送函数
function send_with_retry(ctx: Context, batch: Vec<Event>, retry_limit: int) -> Result<(), Error>:
    for attempt in 0..retry_limit:
        err = send_batch(ctx, batch)
        if err == OK:
            return OK

        // 指数退避
        sleep(Duration(attempt * 100, MILLISECOND))

    return Error("failed after retries")

// 监控队列
function monitor_queue(queue):
    ticker = Ticker(Duration(10, SECOND))
    loop:
        stats = queue.get_stats()
        print("Utilization:", stats.utilization_pct, "%")

        if stats.overflow_count > 0:
            print("Warning: overflow detected")

        if stats.utilization_pct > 80:
            print("High utilization warning")

        ticker.next()
```

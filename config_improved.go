package vinequeue

import (
	"fmt"
	"runtime"
	"time"
)

// Config 队列配置（增强版）
type Config struct {
	// 内存队列配置
	MemorySize         int     `json:"memory_size" yaml:"memory_size" default:"10000"`
	SecondaryQueueSize int     `json:"secondary_queue_size" yaml:"secondary_queue_size" default:"5000"`
	EnableSpillToDisk  bool    `json:"enable_spill_to_disk" yaml:"enable_spill_to_disk" default:"true"`
	MemoryThreshold    float64 `json:"memory_threshold" yaml:"memory_threshold" default:"0.8"`

	// WAL配置
	WALPath           string `json:"wal_path" yaml:"wal_path" default:"./vinequeue.wal"`
	WALMaxSize        int64  `json:"wal_max_size" yaml:"wal_max_size" default:"1073741824"` // 1GB
	WALSyncMode       string `json:"wal_sync_mode" yaml:"wal_sync_mode" default:"periodic"`   // immediate/periodic/async
	WALSyncInterval   time.Duration `json:"wal_sync_interval" yaml:"wal_sync_interval" default:"100ms"`
	EnableCompression bool   `json:"enable_compression" yaml:"enable_compression" default:"true"`
	EnableEncryption  bool   `json:"enable_encryption" yaml:"enable_encryption" default:"false"`
	EncryptionKey     string `json:"encryption_key" yaml:"encryption_key"`

	// 批量发送配置
	BatchSize      int           `json:"batch_size" yaml:"batch_size" default:"1000"`
	MaxBatchSize   int           `json:"max_batch_size" yaml:"max_batch_size" default:"5000"`
	FlushTimeout   time.Duration `json:"flush_timeout" yaml:"flush_timeout" default:"5s"`
	SendTimeout    time.Duration `json:"send_timeout" yaml:"send_timeout" default:"10s"`
	RetryLimit     int           `json:"retry_limit" yaml:"retry_limit" default:"3"`
	RetryBackoff   time.Duration `json:"retry_backoff" yaml:"retry_backoff" default:"1s"`
	MaxRetryBackoff time.Duration `json:"max_retry_backoff" yaml:"max_retry_backoff" default:"30s"`

	// 并发配置
	ProcessorPoolSize int `json:"processor_pool_size" yaml:"processor_pool_size" default:"0"` // 0=auto
	SenderPoolSize    int `json:"sender_pool_size" yaml:"sender_pool_size" default:"0"`       // 0=auto
	MaxConcurrency    int `json:"max_concurrency" yaml:"max_concurrency" default:"100"`

	// 背压控制
	EnableBackpressure     bool    `json:"enable_backpressure" yaml:"enable_backpressure" default:"true"`
	BackpressureThreshold  float64 `json:"backpressure_threshold" yaml:"backpressure_threshold" default:"0.9"`
	BackpressureWindowSize int     `json:"backpressure_window_size" yaml:"backpressure_window_size" default:"100"`
	MaxBackoffMs           uint32  `json:"max_backoff_ms" yaml:"max_backoff_ms" default:"5000"`

	// 熔断器配置
	EnableCircuitBreaker    bool          `json:"enable_circuit_breaker" yaml:"enable_circuit_breaker" default:"true"`
	CircuitBreakerFailures  int           `json:"circuit_breaker_failures" yaml:"circuit_breaker_failures" default:"5"`
	CircuitBreakerSuccess   int           `json:"circuit_breaker_success" yaml:"circuit_breaker_success" default:"2"`
	CircuitBreakerTimeout   time.Duration `json:"circuit_breaker_timeout" yaml:"circuit_breaker_timeout" default:"60s"`

	// 监控配置
	EnableMetrics        bool          `json:"enable_metrics" yaml:"enable_metrics" default:"true"`
	MetricsInterval      time.Duration `json:"metrics_interval" yaml:"metrics_interval" default:"10s"`
	EnableHealthCheck    bool          `json:"enable_health_check" yaml:"enable_health_check" default:"true"`
	HealthCheckInterval  time.Duration `json:"health_check_interval" yaml:"health_check_interval" default:"30s"`

	// 性能调优
	UseZeroCopy      bool `json:"use_zero_copy" yaml:"use_zero_copy" default:"true"`
	UseMmap          bool `json:"use_mmap" yaml:"use_mmap" default:"false"`
	PreallocateBatch bool `json:"preallocate_batch" yaml:"preallocate_batch" default:"true"`

	// 压缩配置
	CompressionType  string `json:"compression_type" yaml:"compression_type" default:"snappy"` // snappy/gzip/lz4/zstd/none
	CompressionLevel int    `json:"compression_level" yaml:"compression_level" default:"1"`    // 1-9 for gzip/zstd
}

// DefaultConfig 返回默认配置（优化版）
func DefaultConfig() Config {
	numCPU := runtime.NumCPU()

	return Config{
		// 内存队列
		MemorySize:         10000,
		SecondaryQueueSize: 5000,
		EnableSpillToDisk:  true,
		MemoryThreshold:    0.8,

		// WAL配置
		WALPath:           "./vinequeue.wal",
		WALMaxSize:        1073741824, // 1GB
		WALSyncMode:       "periodic",
		WALSyncInterval:   100 * time.Millisecond,
		EnableCompression: true,
		EnableEncryption:  false,

		// 批量发送
		BatchSize:       1000,
		MaxBatchSize:    5000,
		FlushTimeout:    5 * time.Second,
		SendTimeout:     10 * time.Second,
		RetryLimit:      3,
		RetryBackoff:    1 * time.Second,
		MaxRetryBackoff: 30 * time.Second,

		// 并发（自动根据CPU核心数调整）
		ProcessorPoolSize: min(numCPU, 4),
		SenderPoolSize:    min(numCPU*2, 8),
		MaxConcurrency:    100,

		// 背压控制
		EnableBackpressure:     true,
		BackpressureThreshold:  0.9,
		BackpressureWindowSize: 100,
		MaxBackoffMs:           5000,

		// 熔断器
		EnableCircuitBreaker:   true,
		CircuitBreakerFailures: 5,
		CircuitBreakerSuccess:  2,
		CircuitBreakerTimeout:  60 * time.Second,

		// 监控
		EnableMetrics:       true,
		MetricsInterval:     10 * time.Second,
		EnableHealthCheck:   true,
		HealthCheckInterval: 30 * time.Second,

		// 性能优化
		UseZeroCopy:      true,
		UseMmap:          false,
		PreallocateBatch: true,

		// 压缩
		CompressionType:  "snappy",
		CompressionLevel: 1,
	}
}

// V4Config 返回V4架构优化配置
func V4Config() Config {
	config := DefaultConfig()

	// V4特定优化
	config.BatchSize = 1000             // V4要求
	config.FlushTimeout = 5 * time.Second // V4要求
	config.MemorySize = 50000           // 更大的内存队列
	config.EnableCompression = true     // 启用压缩减少网络传输
	config.CompressionType = "snappy"   // 最佳性能/压缩比平衡
	config.UseZeroCopy = true           // 零拷贝优化
	config.PreallocateBatch = true      // 预分配批次减少GC

	return config
}

// Validate 验证配置参数（增强版）
func (c *Config) Validate() error {
	// 基础验证
	if c.MemorySize <= 0 {
		return fmt.Errorf("memory_size must be positive, got %d", c.MemorySize)
	}
	if c.BatchSize <= 0 {
		return fmt.Errorf("batch_size must be positive, got %d", c.BatchSize)
	}
	if c.BatchSize > c.MemorySize {
		return fmt.Errorf("batch_size (%d) cannot exceed memory_size (%d)", c.BatchSize, c.MemorySize)
	}
	if c.MaxBatchSize < c.BatchSize {
		c.MaxBatchSize = c.BatchSize * 5
	}
	if c.FlushTimeout <= 0 {
		return fmt.Errorf("flush_timeout must be positive, got %v", c.FlushTimeout)
	}
	if c.SendTimeout <= 0 {
		return fmt.Errorf("send_timeout must be positive, got %v", c.SendTimeout)
	}
	if c.SendTimeout < c.FlushTimeout {
		return fmt.Errorf("send_timeout (%v) should be >= flush_timeout (%v)", c.SendTimeout, c.FlushTimeout)
	}
	if c.RetryLimit < 0 {
		c.RetryLimit = 0
	}
	if c.WALMaxSize <= 0 {
		c.WALMaxSize = 1073741824 // 1GB
	}
	if c.WALPath == "" {
		return fmt.Errorf("wal_path cannot be empty")
	}

	// WAL同步模式验证
	switch c.WALSyncMode {
	case "immediate", "periodic", "async":
		// valid
	default:
		return fmt.Errorf("invalid wal_sync_mode: %s (must be immediate/periodic/async)", c.WALSyncMode)
	}

	// 压缩类型验证
	switch c.CompressionType {
	case "none", "snappy", "gzip", "lz4", "zstd":
		// valid
	default:
		return fmt.Errorf("invalid compression_type: %s", c.CompressionType)
	}

	// 加密验证
	if c.EnableEncryption && c.EncryptionKey == "" {
		return fmt.Errorf("encryption_key required when encryption is enabled")
	}

	// 并发配置自动调整
	if c.ProcessorPoolSize == 0 {
		c.ProcessorPoolSize = min(runtime.NumCPU(), 4)
	}
	if c.SenderPoolSize == 0 {
		c.SenderPoolSize = min(runtime.NumCPU()*2, 8)
	}
	if c.MaxConcurrency <= 0 {
		c.MaxConcurrency = 100
	}

	// 背压阈值验证
	if c.BackpressureThreshold <= 0 || c.BackpressureThreshold > 1 {
		return fmt.Errorf("backpressure_threshold must be between 0 and 1, got %f", c.BackpressureThreshold)
	}

	// 熔断器验证
	if c.CircuitBreakerFailures <= 0 {
		c.CircuitBreakerFailures = 5
	}
	if c.CircuitBreakerSuccess <= 0 {
		c.CircuitBreakerSuccess = 2
	}
	if c.CircuitBreakerTimeout <= 0 {
		c.CircuitBreakerTimeout = 60 * time.Second
	}

	return nil
}

// String 返回配置的字符串表示
func (c Config) String() string {
	return fmt.Sprintf(
		"VineQueue Config: Memory=%d, Batch=%d, Flush=%v, Processors=%d, Senders=%d, Compression=%s",
		c.MemorySize, c.BatchSize, c.FlushTimeout,
		c.ProcessorPoolSize, c.SenderPoolSize, c.CompressionType,
	)
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
package vinequeue

import (
	"time"
)

// Config 队列配置
type Config struct {
	// 内存队列配置
	MemorySize int `json:"memory_size" yaml:"memory_size" default:"10000"`

	// WAL配置
	WALPath     string `json:"wal_path" yaml:"wal_path" default:"./vinequeue.wal"`
	WALMaxSize  int64  `json:"wal_max_size" yaml:"wal_max_size" default:"104857600"` // 100MB
	WALSyncMode string `json:"wal_sync_mode" yaml:"wal_sync_mode" default:"periodic"` // immediate/periodic

	// 批量发送配置
	BatchSize    int           `json:"batch_size" yaml:"batch_size" default:"1000"`
	FlushTimeout time.Duration `json:"flush_timeout" yaml:"flush_timeout" default:"5s"`
	SendTimeout  time.Duration `json:"send_timeout" yaml:"send_timeout" default:"10s"`
	RetryLimit   int           `json:"retry_limit" yaml:"retry_limit" default:"3"`

	// 监控配置
	EnableMetrics bool `json:"enable_metrics" yaml:"enable_metrics" default:"true"`
}

// DefaultConfig 返回默认配置
func DefaultConfig() Config {
	return Config{
		MemorySize:    10000,
		WALPath:       "./vinequeue.wal",
		WALMaxSize:    104857600, // 100MB
		WALSyncMode:   "periodic",
		BatchSize:     1000,
		FlushTimeout:  5 * time.Second,
		SendTimeout:   10 * time.Second,
		RetryLimit:    3,
		EnableMetrics: true,
	}
}

// Validate 验证配置参数
func (c *Config) Validate() error {
	if c.MemorySize <= 0 {
		c.MemorySize = 10000
	}
	if c.BatchSize <= 0 {
		c.BatchSize = 1000
	}
	if c.FlushTimeout <= 0 {
		c.FlushTimeout = 5 * time.Second
	}
	if c.SendTimeout <= 0 {
		c.SendTimeout = 10 * time.Second
	}
	if c.RetryLimit < 0 {
		c.RetryLimit = 3
	}
	if c.WALMaxSize <= 0 {
		c.WALMaxSize = 104857600
	}
	if c.WALPath == "" {
		c.WALPath = "./vinequeue.wal"
	}
	if c.WALSyncMode != "immediate" && c.WALSyncMode != "periodic" {
		c.WALSyncMode = "periodic"
	}
	return nil
}
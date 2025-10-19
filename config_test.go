package vinequeue

// ExtendedConfig 扩展配置（测试和未来功能用）
type ExtendedConfig struct {
	Config

	// 高级功能配置（未来版本）
	EnableSpillToDisk       bool   `json:"enable_spill_to_disk" yaml:"enable_spill_to_disk"`
	EnableCircuitBreaker    bool   `json:"enable_circuit_breaker" yaml:"enable_circuit_breaker"`
	CircuitBreakerFailures  int    `json:"circuit_breaker_failures" yaml:"circuit_breaker_failures"`
	EnableCompression       bool   `json:"enable_compression" yaml:"enable_compression"`
	CompressionType         string `json:"compression_type" yaml:"compression_type"`
	ProcessorPoolSize       int    `json:"processor_pool_size" yaml:"processor_pool_size"`
	SenderPoolSize          int    `json:"sender_pool_size" yaml:"sender_pool_size"`
}

// ToConfig 转换为基础配置
func (ec ExtendedConfig) ToConfig() Config {
	return ec.Config
}

// V4ExtendedConfig 返回V4架构优化的扩展配置（测试用）
func V4ExtendedConfig() ExtendedConfig {
	return ExtendedConfig{
		Config: V4Config(),
		EnableSpillToDisk:      true,
		EnableCircuitBreaker:   false,
		CircuitBreakerFailures: 5,
		EnableCompression:      false,
		CompressionType:        "none",
		ProcessorPoolSize:      4,
		SenderPoolSize:         8,
	}
}
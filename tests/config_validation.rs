use vinequeue::{Config, QueueError};

#[test]
fn config_validation_rejects_invalid_values() {
    let mut config = Config::default_config();
    config.memory_size = 0;
    match config.validate() {
        Err(QueueError::InvalidConfig(message)) => {
            assert_eq!(message, "memory_size must be positive");
        }
        other => panic!("unexpected result: {:?}", other),
    }

    let mut config = Config::default_config();
    config.soft_limit_pct = 90;
    config.hard_limit_pct = 80;
    match config.validate() {
        Err(QueueError::InvalidConfig(message)) => {
            assert_eq!(message, "hard_limit_pct must be greater than soft_limit_pct");
        }
        other => panic!("unexpected result: {:?}", other),
    }

    let mut config = Config::default_config();
    config.batch_size = 0;
    match config.validate() {
        Err(QueueError::InvalidConfig(message)) => {
            assert_eq!(message, "batch_size must be positive");
        }
        other => panic!("unexpected result: {:?}", other),
    }
}

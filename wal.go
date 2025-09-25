package vinequeue

import (
	"bufio"
	"encoding/gob"
	"fmt"
	"io"
	"os"
	"runtime"
	"sync"
	"time"
)

// WALInterface WAL接口定义
type WALInterface[T any] interface {
	Write(data T) error
	ReadAll() ([]T, error)
	Clear() error
	Close() error
	Size() int64
	Path() string
}

// WALEntry WAL条目
type WALEntry[T any] struct {
	Timestamp int64 `gob:"timestamp"`
	Data      T     `gob:"data"`
}

// NewWAL 创建WAL实例（基于平台选择实现）
func NewWAL[T any](path string, maxSize int64, syncMode string) (WALInterface[T], error) {
	if runtime.GOOS == "windows" {
		// Windows使用内存WAL（开发环境）
		return NewMemoryWAL[T](path, maxSize, syncMode)
	}
	// Unix使用磁盘WAL（生产环境）
	return NewDiskWAL[T](path, maxSize, syncMode)
}

// MemoryWAL 内存WAL实现（Windows开发环境）
type MemoryWAL[T any] struct {
	path     string
	maxSize  int64
	syncMode string
	items    []T
	mu       sync.RWMutex
}

// NewMemoryWAL 创建内存WAL
func NewMemoryWAL[T any](path string, maxSize int64, syncMode string) (*MemoryWAL[T], error) {
	return &MemoryWAL[T]{
		path:     path,
		maxSize:  maxSize,
		syncMode: syncMode,
		items:    make([]T, 0),
	}, nil
}

func (w *MemoryWAL[T]) Write(data T) error {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.items = append(w.items, data)
	return nil
}

func (w *MemoryWAL[T]) ReadAll() ([]T, error) {
	w.mu.RLock()
	defer w.mu.RUnlock()
	result := make([]T, len(w.items))
	copy(result, w.items)
	return result, nil
}

func (w *MemoryWAL[T]) Clear() error {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.items = w.items[:0]
	return nil
}

func (w *MemoryWAL[T]) Close() error {
	return nil
}

func (w *MemoryWAL[T]) Size() int64 {
	w.mu.RLock()
	defer w.mu.RUnlock()
	return int64(len(w.items) * 64) // 估算大小
}

func (w *MemoryWAL[T]) Path() string {
	return w.path
}

// DiskWAL 磁盘WAL实现（Unix生产环境）
type DiskWAL[T any] struct {
	file     *os.File
	encoder  *gob.Encoder
	writer   *bufio.Writer
	mu       sync.Mutex
	path     string
	maxSize  int64
	syncMode string
	size     int64
	lastSync time.Time
}

// NewDiskWAL 创建磁盘WAL
func NewDiskWAL[T any](path string, maxSize int64, syncMode string) (*DiskWAL[T], error) {
	wal := &DiskWAL[T]{
		path:     path,
		maxSize:  maxSize,
		syncMode: syncMode,
		lastSync: time.Now(),
	}

	if err := wal.open(); err != nil {
		return nil, fmt.Errorf("open WAL: %w", err)
	}

	return wal, nil
}

func (w *DiskWAL[T]) open() error {
	w.mu.Lock()
	defer w.mu.Unlock()

	file, err := os.OpenFile(w.path, os.O_CREATE|os.O_RDWR|os.O_APPEND, 0644)
	if err != nil {
		return fmt.Errorf("open file: %w", err)
	}

	w.file = file
	stat, err := file.Stat()
	if err != nil {
		return fmt.Errorf("stat file: %w", err)
	}
	w.size = stat.Size()

	w.writer = bufio.NewWriter(file)
	w.encoder = gob.NewEncoder(w.writer)
	return nil
}

func (w *DiskWAL[T]) Write(data T) error {
	w.mu.Lock()
	defer w.mu.Unlock()

	if w.size > w.maxSize {
		return fmt.Errorf("WAL file size exceeded: %d > %d", w.size, w.maxSize)
	}

	entry := WALEntry[T]{
		Timestamp: time.Now().UnixNano(),
		Data:      data,
	}

	if err := w.encoder.Encode(entry); err != nil {
		return fmt.Errorf("encode entry: %w", err)
	}

	if err := w.writer.Flush(); err != nil {
		return fmt.Errorf("flush writer: %w", err)
	}

	w.size += int64(64)

	// 优化：减少同步频率
	if w.syncMode == "immediate" {
		if err := w.file.Sync(); err != nil {
			return fmt.Errorf("sync file: %w", err)
		}
	} else if w.syncMode == "periodic" && time.Since(w.lastSync) > time.Second {
		if err := w.file.Sync(); err != nil {
			return fmt.Errorf("sync file: %w", err)
		}
		w.lastSync = time.Now()
	}

	return nil
}

func (w *DiskWAL[T]) ReadAll() ([]T, error) {
	w.mu.Lock()
	defer w.mu.Unlock()

	file, err := os.Open(w.path)
	if err != nil {
		if os.IsNotExist(err) {
			return []T{}, nil
		}
		return nil, fmt.Errorf("open file for reading: %w", err)
	}
	defer file.Close()

	decoder := gob.NewDecoder(file)
	var entries []T

	for {
		var entry WALEntry[T]
		err := decoder.Decode(&entry)
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("decode entry: %w", err)
		}
		entries = append(entries, entry.Data)
	}

	return entries, nil
}

func (w *DiskWAL[T]) Clear() error {
	w.mu.Lock()
	defer w.mu.Unlock()

	if w.file != nil {
		w.file.Close()
	}

	if err := os.Remove(w.path); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("remove file: %w", err)
	}

	return w.open()
}

func (w *DiskWAL[T]) Close() error {
	w.mu.Lock()
	defer w.mu.Unlock()

	if w.writer != nil {
		if err := w.writer.Flush(); err != nil {
			return fmt.Errorf("flush writer: %w", err)
		}
	}

	if w.file != nil {
		if err := w.file.Sync(); err != nil {
			return fmt.Errorf("sync file: %w", err)
		}
		if err := w.file.Close(); err != nil {
			return fmt.Errorf("close file: %w", err)
		}
	}

	return nil
}

func (w *DiskWAL[T]) Size() int64 {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.size
}

func (w *DiskWAL[T]) Path() string {
	return w.path
}
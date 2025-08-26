# Detailed Implementation Plan: Whisper-rs to OpenAI Whisper CLI Migration

**Project:** Alternator - Mastodon Media Description Bot  
**Branch:** `standalone-whisper`  
**Estimated Timeline:** 3-4 weeks  
**Implementation Date:** August 2025

## Overview

This document provides a detailed, step-by-step implementation plan for migrating from whisper-rs to OpenAI's official Whisper Python CLI implementation with universal AMD+NVIDIA GPU support.

## Phase 1: Environment Setup & Docker Configuration
**Duration:** 2-3 days  
**Priority:** High

### 1.1 Update Docker Base Configuration
- [ ] **File:** `Dockerfile`
  - [ ] Add Python 3.11+ installation
  - [ ] Install pip and essential build tools
  - [ ] Install OpenAI Whisper package
  - [ ] Verify FFmpeg compatibility with Whisper CLI
- [ ] **File:** `Dockerfile.optimized`
  - [ ] Apply same Python/Whisper setup to optimized build
  - [ ] Ensure multi-stage build efficiency

### 1.2 Universal GPU Support Implementation
- [ ] **File:** `Dockerfile`
  - [ ] Install PyTorch with CUDA support (NVIDIA)
  - [ ] Install PyTorch with ROCm support (AMD)
  - [ ] Add runtime GPU detection script
  - [ ] Test container size impact (~3GB increase expected)
- [ ] **File:** `scripts/setup-gpu.sh` (Optional alternative approach)
  - [ ] Create runtime GPU detection script
  - [ ] Implement dynamic PyTorch installation
  - [ ] Add fallback to CPU-only mode

### 1.3 Environment Testing
- [ ] **Local Testing**
  - [ ] Build Docker image with dual GPU support
  - [ ] Test Whisper CLI installation and basic functionality
  - [ ] Verify GPU detection works correctly
  - [ ] Test model download and caching
- [ ] **GPU Testing**
  - [ ] Test with NVIDIA GPU (if available)
  - [ ] Test with AMD GPU (if available)
  - [ ] Test CPU fallback mode
  - [ ] Benchmark performance differences

**Deliverables:**
- Updated Dockerfile with Python/Whisper/PyTorch
- GPU detection and setup scripts
- Docker image size analysis report
- Basic GPU functionality test results

---

## Phase 2: Create WhisperCli Implementation
**Duration:** 4-5 days  
**Priority:** High

### 2.1 Core WhisperCli Structure
- [ ] **File:** `src/whisper_cli.rs` (new file)
  - [ ] Define `WhisperCli` struct with all required fields
  - [ ] Implement `new()` constructor with config validation
  - [ ] Add proper error handling with `MediaError` integration
  - [ ] Create helper methods for path handling

### 2.2 GPU Detection Implementation
- [ ] **Function:** `detect_optimal_device()`
  - [ ] Implement NVIDIA GPU detection (`nvidia-smi`)
  - [ ] Implement AMD GPU detection (`rocm-smi`)
  - [ ] Add CPU fallback logic
  - [ ] Include proper logging for device selection
  - [ ] Test detection accuracy on different systems

### 2.3 Model Preloading System
- [ ] **Function:** `preload_model()`
  - [ ] Implement direct Whisper model loading (no dummy audio)
  - [ ] Add GPU context warmup with minimal tensor operations
  - [ ] Support custom `model_dir` via `download_root` parameter
  - [ ] Implement proper error handling and logging
  - [ ] Add atomic boolean tracking for preload status
- [ ] **Testing**
  - [ ] Test preloading with different models (tiny, medium, large)
  - [ ] Test custom model directory support
  - [ ] Test GPU warmup functionality
  - [ ] Measure preloading performance impact

### 2.4 Transcription Implementation
- [ ] **Function:** `transcribe_audio()`
  - [ ] Implement Whisper CLI command construction
  - [ ] Add `--model_dir` support for existing configuration
  - [ ] Support language specification and auto-detection
  - [ ] Implement proper output file handling
  - [ ] Add comprehensive error handling
- [ ] **CLI Integration**
  - [ ] Test all Whisper CLI options used
  - [ ] Verify output format compatibility
  - [ ] Test with various audio formats
  - [ ] Validate transcript reading and cleanup

### 2.5 Integration Points
- [ ] **File:** `src/lib.rs`
  - [ ] Add `whisper_cli` module declaration
  - [ ] Export necessary types and functions
- [ ] **Dependencies**
  - [ ] Add required imports (tokio, std::process, etc.)
  - [ ] Update error types if needed
  - [ ] Test compilation and basic functionality

**Deliverables:**
- Complete `WhisperCli` implementation
- GPU detection and device selection
- Model preloading system
- CLI transcription functionality
- Unit tests for core functions

---

## Phase 3: Update Configuration Structure
**Duration:** 2 days  
**Priority:** High

### 3.1 WhisperConfig Enhancement
- [ ] **File:** `src/config.rs`
  - [ ] Add new optional fields (python_executable, device, backend, preload)
  - [ ] Maintain all existing fields for backward compatibility
  - [ ] Update `Default` implementation with sensible defaults
  - [ ] Add proper documentation for new fields

### 3.2 Configuration Validation
- [ ] **Function:** `validate_whisper_config()`
  - [ ] Validate model names against supported list
  - [ ] Check Python executable availability
  - [ ] Validate model_dir path if specified
  - [ ] Add device option validation
- [ ] **Migration Support**
  - [ ] Ensure existing configs work without changes
  - [ ] Test config parsing with new optional fields
  - [ ] Verify default value assignment

### 3.3 Configuration Integration
- [ ] **File:** `alternator.toml.example`
  - [ ] Update example configuration with new options
  - [ ] Add comments explaining GPU options
  - [ ] Provide examples for different use cases
- [ ] **Testing**
  - [ ] Test config loading with existing files
  - [ ] Test config loading with new fields
  - [ ] Test missing field handling (defaults)
  - [ ] Validate backward compatibility

**Deliverables:**
- Enhanced WhisperConfig structure
- Configuration validation
- Updated example configuration
- Backward compatibility verification

---

## Phase 4: Integrate CLI into Audio Processing
**Duration:** 3-4 days  
**Priority:** High

### 4.1 Audio Processing Integration
- [ ] **File:** `src/media/audio.rs`
  - [ ] Replace whisper-rs calls with WhisperCli calls
  - [ ] Update function signatures to use new implementation
  - [ ] Maintain existing audio preprocessing logic
  - [ ] Add proper error handling and conversion
- [ ] **Function:** `transcribe_audio_internal()`
  - [ ] Replace whisper-rs context creation
  - [ ] Integrate WhisperCli transcription call
  - [ ] Maintain language detection and configuration
  - [ ] Preserve existing audio format support

### 4.2 Startup Integration
- [ ] **File:** `src/main.rs`
  - [ ] Add WhisperCli initialization
  - [ ] Implement model preloading during startup
  - [ ] Add proper error handling for initialization
  - [ ] Integrate with existing service startup flow
- [ ] **Application State**
  - [ ] Add WhisperCli to application state/context
  - [ ] Ensure proper sharing across async tasks
  - [ ] Handle initialization failures gracefully

### 4.3 Error Handling Updates
- [ ] **File:** `src/error.rs`
  - [ ] Add new error variants for CLI failures
  - [ ] Update error messages for user clarity
  - [ ] Maintain error context propagation
- [ ] **Error Recovery**
  - [ ] Implement fallback strategies
  - [ ] Add retry logic for transient failures
  - [ ] Improve error reporting and logging

### 4.4 Dependencies Cleanup
- [ ] **File:** `Cargo.toml`
  - [ ] Remove `whisper-rs` dependency
  - [ ] Add any new Rust dependencies if needed
  - [ ] Update dependency versions for compatibility
- [ ] **File:** `src/whisper.rs`
  - [ ] Remove whisper-rs model management code
  - [ ] Keep any reusable utility functions
  - [ ] Update or remove related tests

**Deliverables:**
- Updated audio processing with CLI integration
- Application startup with model preloading
- Enhanced error handling
- Cleaned up dependencies

---

## Phase 5: Testing and Validation
**Duration:** 5-6 days  
**Priority:** High

### 5.1 Unit Testing
- [ ] **File:** `tests/whisper_cli_tests.rs` (new)
  - [ ] Test GPU detection logic
  - [ ] Test model preloading functionality
  - [ ] Test transcription with various inputs
  - [ ] Test error handling scenarios
  - [ ] Test configuration validation
- [ ] **File:** `src/media/audio.rs` (update tests)
  - [ ] Update existing audio processing tests
  - [ ] Test CLI integration points
  - [ ] Test language detection with new backend
  - [ ] Test error scenarios and fallbacks

### 5.2 Integration Testing
- [ ] **File:** `tests/integration_tests.rs` (update)
  - [ ] Test full pipeline with WhisperCli
  - [ ] Test various audio formats and sizes
  - [ ] Test different model sizes
  - [ ] Test custom model_dir functionality
  - [ ] Test startup and shutdown procedures
- [ ] **Docker Testing**
  - [ ] Test full Docker build and run
  - [ ] Test GPU detection in containers
  - [ ] Test model persistence with volumes
  - [ ] Test performance with different GPU types

### 5.3 Performance Testing
- [ ] **Benchmarking**
  - [ ] Compare performance vs whisper-rs
  - [ ] Test GPU acceleration effectiveness
  - [ ] Measure startup time with preloading
  - [ ] Test memory usage patterns
  - [ ] Document performance characteristics
- [ ] **Load Testing**
  - [ ] Test concurrent transcription requests
  - [ ] Test long-running stability
  - [ ] Test resource cleanup and leaks
  - [ ] Validate thread safety

### 5.4 Compatibility Testing
- [ ] **Configuration Testing**
  - [ ] Test with existing configuration files
  - [ ] Test migration from old to new format
  - [ ] Test all configuration combinations
  - [ ] Validate default behavior preservation
- [ ] **Audio Format Testing**
  - [ ] Test all supported audio formats
  - [ ] Test edge cases (very short/long files)
  - [ ] Test different sample rates and channels
  - [ ] Validate output quality and accuracy

**Deliverables:**
- Comprehensive test suite
- Performance benchmark results
- Integration test coverage
- Compatibility verification report

---

## Phase 6: Documentation and Cleanup
**Duration:** 2-3 days  
**Priority:** Medium

### 6.1 Code Documentation
- [ ] **File:** `src/whisper_cli.rs`
  - [ ] Add comprehensive rustdoc comments
  - [ ] Document all public functions and structs
  - [ ] Add usage examples in documentation
  - [ ] Document error conditions and handling
- [ ] **File:** `src/config.rs`
  - [ ] Update configuration documentation
  - [ ] Document new fields and their effects
  - [ ] Add migration guidance in comments

### 6.2 User Documentation
- [ ] **File:** `README.md`
  - [ ] Update installation requirements (Python/PyTorch)
  - [ ] Add GPU setup instructions
  - [ ] Update configuration examples
  - [ ] Add troubleshooting section for common issues
- [ ] **File:** `CHANGELOG.md`
  - [ ] Document breaking changes (if any)
  - [ ] List new features and improvements
  - [ ] Add migration instructions
  - [ ] Document performance improvements

### 6.3 Migration Guide
- [ ] **File:** `MIGRATION_GUIDE.md` (new)
  - [ ] Step-by-step migration instructions
  - [ ] Configuration migration examples
  - [ ] Docker setup updates
  - [ ] Troubleshooting common migration issues
  - [ ] Performance tuning recommendations

### 6.4 Final Cleanup
- [ ] **Code Review**
  - [ ] Remove any dead code or unused imports
  - [ ] Clean up temporary debugging code
  - [ ] Ensure consistent code style
  - [ ] Verify all TODOs are addressed
- [ ] **Build Verification**
  - [ ] Test clean build from scratch
  - [ ] Verify all tests pass
  - [ ] Check clippy lints and warnings
  - [ ] Validate Docker image functionality

**Deliverables:**
- Complete code documentation
- Updated user documentation
- Migration guide for users
- Clean, production-ready codebase

---

## Implementation Schedule

### Week 1: Foundation
- Days 1-3: Phase 1 (Docker & Environment Setup)
- Days 4-5: Phase 2 start (WhisperCli structure)

### Week 2: Core Implementation  
- Days 1-3: Phase 2 completion (WhisperCli functionality)
- Days 4-5: Phase 3 (Configuration updates)

### Week 3: Integration
- Days 1-4: Phase 4 (Audio processing integration)
- Day 5: Phase 5 start (Basic testing)

### Week 4: Testing & Polish
- Days 1-3: Phase 5 completion (Comprehensive testing)
- Days 4-5: Phase 6 (Documentation & cleanup)

## Risk Mitigation

### Technical Risks
- **GPU Detection Failures**: Implement robust fallback to CPU
- **Performance Regression**: Benchmark early and optimize hotspots
- **Docker Size Issues**: Consider multi-stage builds and optional components
- **Configuration Compatibility**: Maintain extensive backward compatibility tests

### Timeline Risks
- **Complex GPU Setup**: Allow extra time for cross-platform testing
- **Integration Challenges**: Plan for potential audio processing complications
- **Testing Overhead**: Allocate sufficient time for comprehensive validation

## Success Criteria

### Functional Requirements
- [ ] All existing Whisper functionality preserved
- [ ] GPU acceleration working on AMD and NVIDIA
- [ ] Model preloading reduces subsequent transcription time
- [ ] Backward compatibility with existing configurations
- [ ] Docker image builds and runs successfully

### Performance Requirements
- [ ] GPU transcription 5-40x faster than CPU
- [ ] Startup time acceptable with model preloading
- [ ] Memory usage comparable or better than whisper-rs
- [ ] No regression in transcription accuracy

### Quality Requirements
- [ ] All tests passing (unit, integration, performance)
- [ ] Clean code with comprehensive documentation
- [ ] Production-ready error handling and logging
- [ ] User-friendly migration process

---

**Implementation Lead:** AI Assistant  
**Review Required:** Before each phase completion  
**Testing:** Continuous throughout development  
**Documentation:** Updated with each phase
# Whisper CPU Compatibility Fix

## Problem
The `whisper-rs` library can cause "invalid opcode" errors on older or virtualized CPUs that don't support modern SIMD instructions like AVX, AVX2, or FMA. This manifests as:

```
traps: tokio-runtime-w[3117] trap invalid opcode ip:55ff48657520 sp:7f16ab3157f0 error:0 in alternator[64e520,55ff480b3000+5f6000]
```

## Solution
This fix implements multiple layers of CPU compatibility:

### 1. Runtime Environment Variables
The application now sets these environment variables at startup to disable hardware acceleration:
- `GGML_NO_CUBLAS=1` - Disable CUDA acceleration
- `GGML_NO_METAL=1` - Disable Metal (macOS GPU) acceleration  
- `GGML_NO_ACCELERATE=1` - Disable Accelerate framework
- `GGML_NO_OPENBLAS=1` - Disable OpenBLAS acceleration

### 2. Whisper Context Configuration
The WhisperContext is initialized with conservative parameters:
- GPU acceleration explicitly disabled via `use_gpu(false)`
- Safer defaults for CPU-only operation

### 3. Docker Build Optimization
For production deployments, consider building with CPU-specific targets:

```dockerfile
# For maximum compatibility (x86-64 baseline)
ENV RUSTFLAGS="-C target-cpu=x86-64"

# For modern but compatible (x86-64-v2 with SSE4.2, but no AVX)
ENV RUSTFLAGS="-C target-cpu=x86-64-v2"
```

## Testing
After applying this fix:
1. The binary should run on older CPUs without illegal instruction errors
2. Whisper functionality should work with reduced performance but better compatibility
3. Docker containers should be more portable across different host architectures

## Performance Impact
- Audio transcription will be slower without SIMD optimizations
- Memory usage may be slightly higher
- CPU compatibility is significantly improved

## Files Modified
- `src/main.rs` - Added CPU safety initialization
- `src/media/audio.rs` - Updated WhisperContext creation
- `src/media/video.rs` - Updated WhisperContext creation
- `Dockerfile` - Improved build compatibility

## Environment Variables
You can also set these manually if needed:
```bash
export GGML_NO_CUBLAS=1
export GGML_NO_METAL=1
export GGML_NO_ACCELERATE=1
export GGML_NO_OPENBLAS=1
```
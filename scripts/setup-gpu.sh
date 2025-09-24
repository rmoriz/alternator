#!/bin/bash
set -e

# Runtime GPU Detection and PyTorch Installation Script
# Alternative approach for smaller base images

echo "🔍 Detecting GPU hardware..."

# Function to detect NVIDIA GPU
detect_nvidia() {
    if command -v nvidia-smi &> /dev/null; then
        if nvidia-smi &> /dev/null; then
            echo "✅ NVIDIA GPU detected"
            nvidia-smi --query-gpu=name,driver_version --format=csv,noheader,nounits
            return 0
        fi
    fi
    return 1
}

# Function to detect AMD GPU
detect_amd() {
    if command -v rocm-smi &> /dev/null; then
        if rocm-smi &> /dev/null; then
            echo "✅ AMD GPU detected"
            rocm-smi --showproductname
            return 0
        fi
    fi
    # Alternative check for AMD GPU via lspci
    if command -v lspci &> /dev/null; then
        if lspci | grep -i "amd\|ati" | grep -i "vga\|display\|3d" &> /dev/null; then
            echo "✅ AMD GPU detected (via lspci)"
            return 0
        fi
    fi
    return 1
}

# Main GPU detection and installation logic
main() {
    local gpu_detected=false
    
    # Check for NVIDIA GPU first (higher priority)
    if detect_nvidia; then
        echo "🚀 Installing CUDA PyTorch for NVIDIA GPU..."
        pip3 install --no-cache-dir torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu118
        gpu_detected=true
    elif detect_amd; then
        echo "🚀 Installing ROCm PyTorch for AMD GPU..."
        pip3 install --no-cache-dir torch torchvision torchaudio --index-url https://download.pytorch.org/whl/rocm5.4.2
        gpu_detected=true
    fi
    
    # Fallback to CPU-only if no GPU detected
    if [ "$gpu_detected" = false ]; then
        echo "💻 No GPU detected, installing CPU-only PyTorch..."
        pip3 install --no-cache-dir torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cpu
    fi
    
    # Verify PyTorch installation
    echo "🔧 Verifying PyTorch installation..."
    python3 -c "
import torch
print(f'PyTorch version: {torch.__version__}')
print(f'CUDA available: {torch.cuda.is_available()}')
if torch.cuda.is_available():
    print(f'CUDA version: {torch.version.cuda}')
    print(f'GPU count: {torch.cuda.device_count()}')
    for i in range(torch.cuda.device_count()):
        print(f'GPU {i}: {torch.cuda.get_device_name(i)}')
else:
    print('Using CPU backend')
"
    
    # Verify Deno installation
    echo "🔧 Verifying Deno installation..."
    if command -v deno &> /dev/null; then
        echo "✅ Deno $(deno --version | head -n1 | cut -d' ' -f2) is available"
    else
        echo "⚠️  Deno not found - installing..."
        curl -fsSL https://deno.land/install.sh | DENO_INSTALL=/usr/local sh
        chmod +x /usr/local/bin/deno
        echo "✅ Deno installed successfully"
    fi
    
    echo "✅ GPU setup completed successfully!"
}

# Run main function
main "$@"
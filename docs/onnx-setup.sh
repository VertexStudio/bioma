#!/bin/bash

# Set the installation directory
INSTALL_DIR="$HOME"  # Change this to "." for current directory if preferred

# ONNX Runtime version
ONNX_VERSION="1.20.0"

# Download and install onnxruntime, if not already installed
ONNX_DIR="$INSTALL_DIR/onnxruntime-linux-x64-gpu-$ONNX_VERSION"
ONNX_TAR="onnxruntime-linux-x64-gpu-$ONNX_VERSION.tgz"
ONNX_URL="https://github.com/microsoft/onnxruntime/releases/download/v$ONNX_VERSION/$ONNX_TAR"

if [ ! -d "$ONNX_DIR" ]; then
    echo "Downloading onnxruntime"
    curl -L "$ONNX_URL" -o "$ONNX_TAR"
    tar -xzf "$ONNX_TAR" -C "$INSTALL_DIR"
    rm "$ONNX_TAR"
fi

# Add to .bashrc if not already present
ONNX_LIB_PATH="$ONNX_DIR/lib"

if ! grep -q "$ONNX_LIB_PATH" ~/.bashrc; then
    echo "# ONNX Runtime" >> ~/.bashrc
    echo "export LD_LIBRARY_PATH=\"\$LD_LIBRARY_PATH:$ONNX_LIB_PATH\"" >> ~/.bashrc
    echo "ONNX Runtime path added to .bashrc"
else
    echo "ONNX Runtime path already in .bashrc"
fi

# Source .bashrc to apply changes immediately
source ~/.bashrc

echo "Script completed. ONNX Runtime installed and path added to LD_LIBRARY_PATH."
echo "Please restart your terminal or run 'source ~/.bashrc' to apply changes."
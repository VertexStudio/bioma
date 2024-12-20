import os
from markitdown import MarkItDown
from flask import Flask, request, jsonify

app = Flask(__name__)
markitdown = MarkItDown()

@app.route('/convert', methods=['POST'])
def convert():
    if 'file' not in request.files:
        return jsonify({"error": "No file part in the request"}), 400

    file = request.files['file']
    if file.filename == '':
        return jsonify({"error": "No file selected"}), 400

    # Save the file to a temporary location
    temp_path = os.path.join('/tmp', file.filename)
    file.save(temp_path)

    try:
        # Use MarkItDown to process the file
        result = markitdown.convert(temp_path)
        if result is None:
            return jsonify({"error": "Failed to process the file"}), 500

        return jsonify({"text_content": result.text_content})
    finally:
        # Clean up the temporary file
        if os.path.exists(temp_path):
            os.remove(temp_path)

if __name__ == '__main__':
    port = int(os.getenv('PORT', 5000))  # Default to 5000 if PORT is not set
    app.run(host='0.0.0.0', port=port)
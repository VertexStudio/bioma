# from flask import Flask, request, jsonify
# from markitdown import MarkItDown

# app = Flask(__name__)
# markitdown = MarkItDown()

# @app.route('/convert', methods=['POST'])
# def convert():
#     try:
#         print("Received request", request.json)
#         data = request.json  # Parse JSON data
#         print("Received JSON data:", data)
#         file_path = data.get('file_path') 
#         print("Got file", file_path)
#         result = markitdown.convert(file_path)
#         print("Converted file", result)
#         return jsonify({"text_content": result.text_content})
#     except Exception as e:
#         print("Error: ", e)
#         return jsonify({"error": str(e)})
    

# if __name__ == '__main__':
#     app.run(host='0.0.0.0', port=5000)

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
    app.run(host='0.0.0.0', port=5000)
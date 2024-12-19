from flask import Flask, request, jsonify
from markitdown import MarkItDown

app = Flask(__name__)
markitdown = MarkItDown()

@app.route('/convert', methods=['POST'])
def convert():
    try:
        print("Received request", request.json)
        data = request.json  # Parse JSON data
        print("Received JSON data:", data)
        file_path = data.get('file_path') 
        print("Got file", file_path)
        result = markitdown.convert(file_path)
        print("Converted file", result)
        return jsonify({"text_content": result.text_content})
    except Exception as e:
        print("Error: ", e)
        return jsonify({"error": str(e)})
    

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=5000)
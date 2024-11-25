from transformers import AutoTokenizer, AutoModelForSequenceClassification
import torch
from flask import Flask, request, jsonify
import threading
import json

# Load the tokenizer and model
model_name = "BAAI/bge-reranker-v2-m3"
tokenizer = AutoTokenizer.from_pretrained(model_name)
model = AutoModelForSequenceClassification.from_pretrained(model_name)
model.eval()  # Set the model to evaluation mode

app = Flask(__name__)

# To handle threading issues with PyTorch and Flask
model_lock = threading.Lock()


def rerank(query, texts):
    # Prepare the inputs
    inputs = tokenizer(
        [query] * len(texts), texts, return_tensors="pt", padding=True, truncation=True
    )
    # Get model predictions
    with torch.no_grad():
        with model_lock:
            outputs = model(**inputs)
    scores = outputs.logits.squeeze(-1)
    # Pair indices with their scores
    ranked_texts = [
        {"index": idx, "score": score.item()} for idx, score in enumerate(scores)
    ]
    return ranked_texts


@app.route("/rerank", methods=["POST"])
def rerank_endpoint():
    data = request.get_json()
    query = data.get("query")
    texts = data.get("texts")
    raw_scores = data.get("raw_scores", False)

    if query is None or texts is None:
        return jsonify({"error": "Please provide a query and a list of texts"}), 400

    ranked_texts = rerank(query, texts)

    # if raw_scores is false, normalize the scores
    if not raw_scores:
        min_score = min(text["score"] for text in ranked_texts)
        max_score = max(text["score"] for text in ranked_texts)
        score_range = max_score - min_score
        if score_range > 0:
            for text in ranked_texts:
                text["score"] = (text["score"] - min_score) / score_range
        else:
            for text in ranked_texts:
                text["score"] = 1.0

    # Print pretty JSON for debugging
    print("\nRanked texts:")
    print(json.dumps(ranked_texts, indent=2))

    # Return the ranked texts as a JSON array
    return jsonify(ranked_texts)


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=9124)

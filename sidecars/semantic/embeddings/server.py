"""Local-only model service. Explicit CUDA requests fail rather than silently using CPU."""
import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import torch
from sentence_transformers import SentenceTransformer

MODEL = "sentence-transformers/all-MiniLM-L6-v2"
REVISION = "1110a243fdf4706b3f48f1d95db1a4f5529b4d41"
requested = os.getenv("EMBEDDING_DEVICE", "cpu")
if requested not in ("cpu", "cuda", "auto"):
    raise RuntimeError("EMBEDDING_DEVICE must be cpu, cuda, or auto")
if requested == "cuda" and not torch.cuda.is_available():
    raise RuntimeError("CUDA requested but unavailable; enable GPU access or select CPU explicitly")
device = "cuda" if requested != "cpu" and torch.cuda.is_available() else "cpu"
model = SentenceTransformer(MODEL, revision=REVISION, device=device, trust_remote_code=False,
                            model_kwargs={"use_safetensors": True})
model.eval()
profile = {"model": MODEL, "revision": REVISION, "dimension": model.get_sentence_embedding_dimension(),
           "chunk_tokens": 224, "overlap_tokens": 32, "pooling": "normalized-mean-of-normalized-windows-v1"}
gate = threading.BoundedSemaphore(1)
QUEUE_SECONDS = float(os.environ.get("EMBEDDING_QUEUE_SECONDS", "30"))


def embed(texts):
    # Cover long literals instead of silently truncating at the model's token limit.
    windows, spans = [], []
    for text in texts:
        tokens = model.tokenizer.encode(text, add_special_tokens=False)
        start = len(windows)
        if not tokens:
            windows.append("")
        for offset in range(0, len(tokens), 192):
            windows.append(model.tokenizer.decode(tokens[offset:offset + 224], skip_special_tokens=True))
            if offset + 224 >= len(tokens):
                break
        spans.append((start, len(windows)))
    with torch.inference_mode():
        vectors = model.encode(windows, batch_size=32, convert_to_tensor=True,
                               normalize_embeddings=True, show_progress_bar=False)
        pooled = torch.stack([vectors[start:end].mean(dim=0) for start, end in spans])
        pooled = torch.nn.functional.normalize(pooled, p=2, dim=1)
        if not torch.isfinite(pooled).all():
            raise RuntimeError("model produced non-finite vectors")
        return pooled.cpu().tolist()


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, value):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path != "/health":
            return self.reply(404, {"error": "not found"})
        self.reply(200, {"profile": profile, "device": device, "torch": torch.__version__,
                         "gpu": torch.cuda.get_device_name(0) if device == "cuda" else None})

    def do_POST(self):
        if self.path != "/embed":
            return self.reply(404, {"error": "not found"})
        if self.headers.get_content_type() != "application/json":
            return self.reply(415, {"error": "expected application/json"})
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 4 * 1024 * 1024:
                return self.reply(413, {"error": "request must be 1 byte to 4 MiB"})
            request = json.loads(self.rfile.read(length))
            texts = request.get("texts") if isinstance(request, dict) else None
            if not isinstance(texts, list) or not 1 <= len(texts) <= 64:
                return self.reply(400, {"error": "texts must contain 1 to 64 strings"})
            if any(not isinstance(text, str) or not text.strip() or len(text.encode()) > 65536 for text in texts):
                return self.reply(400, {"error": "each text must be nonempty and at most 64 KiB"})
        except (ValueError, UnicodeError):
            return self.reply(400, {"error": "invalid JSON request"})
        # Queue briefly instead of rejecting: federated queries can issue the same search several times at once.
        if not gate.acquire(timeout=QUEUE_SECONDS):
            return self.reply(429, {"error": "embedding operation capacity exhausted"})
        try:
            self.reply(200, {"profile": profile, "device": device, "vectors": embed(texts)})
        except Exception as error:
            self.reply(500, {"error": str(error)})
        finally:
            gate.release()


if __name__ == "__main__":
    print(json.dumps({"ready": True, "profile": profile, "device": device}), flush=True)
    ThreadingHTTPServer(("0.0.0.0", 8081), Handler).serve_forever()

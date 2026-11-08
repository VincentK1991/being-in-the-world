import importlib.metadata
import json
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

IDENTITY = {'engine': 'owlrl', 'version': importlib.metadata.version('owlrl'),
            'rdflib': importlib.metadata.version('rdflib'), 'protocol': 1,
            'profiles': ['rdfs-v1', 'owlrl-core-v1'],
            'axiomatic_triples': False, 'datatype_axioms': False}
GATE = threading.BoundedSemaphore(1)
class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body):
        body = json.dumps(body).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        self.reply(200, IDENTITY) if self.path == '/health' else self.reply(404, {'error':'not found'})
    def do_POST(self):
        if self.path != '/infer':
            return self.reply(404, {'error':'not found'})
        if self.headers.get_content_type() != 'application/json':
            return self.reply(415, {'error':'expected application/json'})
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= 32 * 1024 * 1024:
                return self.reply(413, {'error':'body exceeds limit'})
            body = self.rfile.read(length)
            request = json.loads(body)
            if set(request) != {'profile', 'ntriples'}:
                return self.reply(400, {'error':'expected profile and ntriples'})
        except (ValueError, TypeError):
            return self.reply(400, {'error':'invalid JSON'})
        if not GATE.acquire(blocking=False):
            return self.reply(429, {'error':'reasoner busy'})
        try:
            result = subprocess.run(['python', 'worker.py'], input=body, capture_output=True, timeout=60)
            response = json.loads(result.stdout)
            if result.returncode:
                return self.reply(422, response)
            response['reasoner'] = IDENTITY
            self.reply(200, response)
        except subprocess.TimeoutExpired:
            self.reply(504, {'error':'reasoning exceeded 60 seconds; worker terminated'})
        except Exception:
            self.reply(500, {'error':'reasoning worker failed'})
        finally:
            GATE.release()
if __name__ == '__main__':
    ThreadingHTTPServer(('0.0.0.0', 8082), Handler).serve_forever()

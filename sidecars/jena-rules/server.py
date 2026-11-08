import json
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
IDENTITY={'engine':'jena','version':'6.2.0','protocol':1,'profiles':['jena-forward-v1'],'mode':'forwardRETE','rules':'positive-range-restricted-triples-v1'}
GATE=threading.BoundedSemaphore(1)
class Handler(BaseHTTPRequestHandler):
    def reply(self,status,value):
        body=json.dumps(value).encode(); self.send_response(status)
        self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        self.reply(200,IDENTITY) if self.path=='/health' else self.reply(404,{'error':'not found'})
    def do_POST(self):
        if self.path!='/infer': return self.reply(404,{'error':'not found'})
        if self.headers.get_content_type()!='application/json': return self.reply(415,{'error':'expected application/json'})
        try:
            length=int(self.headers.get('Content-Length','0'))
            if not 0<length<=32*1024*1024: return self.reply(413,{'error':'body exceeds limit'})
            body=self.rfile.read(length); value=json.loads(body)
            if not isinstance(value,dict) or set(value)!={'profile','ntriples','rules'}: return self.reply(400,{'error':'expected profile, ntriples and rules'})
        except (ValueError,TypeError): return self.reply(400,{'error':'invalid JSON'})
        if not GATE.acquire(blocking=False): return self.reply(429,{'error':'reasoner busy'})
        try:
            worker=subprocess.run(['java','-Xmx384m','-cp','classes:lib/*','RuleWorker'],input=body,capture_output=True,timeout=60)
            result=json.loads(worker.stdout)
            if worker.returncode: return self.reply(422,result)
            if result.pop('engine_version')!=IDENTITY['version']: raise ValueError('version mismatch')
            result['reasoner']=IDENTITY; self.reply(200,result)
        except subprocess.TimeoutExpired: self.reply(504,{'error':'reasoning exceeded 60 seconds; worker terminated'})
        except Exception: self.reply(500,{'error':'reasoning worker failed'})
        finally: GATE.release()
if __name__=='__main__': ThreadingHTTPServer(('0.0.0.0',8083),Handler).serve_forever()

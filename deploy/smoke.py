#!/usr/bin/env python3
"""Real HTTP/S3/native pack+schedule check. Uses repository fixture, no external user files.
Creates guest tasks and deletes them after success; does not touch existing jobs or volumes.
"""
import argparse, hashlib, http.cookiejar, json, pathlib, time, urllib.request, urllib.error, uuid

class Client:
    def __init__(self, origin):
        self.origin=origin.rstrip('/');self.jar=http.cookiejar.CookieJar()
        self.http=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
        self.call('GET','/auth/csrf');self.call('POST','/auth/guest')
    def call(self, method, path, data=None, headers=None):
        h={'Origin':self.origin}
        if method not in ('GET','HEAD'):
            h['X-XSRF-TOKEN']=next(c.value for c in self.jar if c.name=='XSRF-TOKEN')
        if data is not None:
            if isinstance(data,bytes):h['Content-Type']='application/octet-stream'
            else:h['Content-Type']='application/json';data=json.dumps(data).encode()
        h.update(headers or {})
        request=urllib.request.Request(self.origin+'/api/v2'+path,data=data,method=method,headers=h)
        try:
            with self.http.open(request,timeout=120) as response:
                body=response.read();return json.loads(body) if body else None
        except urllib.error.HTTPError as e:raise RuntimeError(f'{method} {path}: HTTP{e.code} {e.read().decode()[:500]}') from e
    def upload(self, path):
        c=self.call('POST','/challenges');nonce=0
        while int.from_bytes(hashlib.sha256(f'{c["id"]}:{c["salt"]}:{nonce}'.encode()).digest(),'big') >= 1 << (256-c['difficulty']):nonce+=1
        data=path.read_bytes()
        u=self.call('POST','/uploads',{'name':path.name,'sizeBytes':len(data),'challengeId':c['id'],'nonce':str(nonce)})
        return self.call('PUT',f'/uploads/{u["fileId"]}/content',data)
    def job(self, body):return self.call('POST','/jobs',body,{'Idempotency-Key':str(uuid.uuid4())})
    def wait(self, id, timeout):
        deadline=time.monotonic()+timeout;previous=None
        while time.monotonic()<deadline:
            job=self.call('GET',f'/jobs/{id}?debug=true')
            state=(job['status'],job['phase'],job['progressPercent'])
            if state!=previous:print(id,*state,flush=True);previous=state
            if job['status']=='SUCCEEDED':return job
            if job['status'] not in ('QUEUED','RUNNING','RETRY_WAIT','LOST','FINALIZING'):raise RuntimeError(json.dumps(job,ensure_ascii=False))
            time.sleep(2)
        raise TimeoutError(f'Job {id} did not finish')
    def verify_downloads(self, job):
        assert any(a['name'].endswith('.3mf') for a in job['artifacts'])
        for a in job['artifacts']:
            if not a['name'].endswith('.3mf'):continue
            d=self.call('POST',f'/jobs/{job["jobId"]}/artifacts/{a["id"]}/download')
            with urllib.request.urlopen(d['url'],timeout=60) as response:
                data=response.read();assert len(data)==a['sizeBytes'] and data[:2]==b'PK'
        print('S3 signed 3MF download verified',flush=True)

def main():
    p=argparse.ArgumentParser();p.add_argument('--url',default='http://127.0.0.1:8088');p.add_argument('--timeout',type=int,default=900)
    p.add_argument('--pack-only',action='store_true');p.add_argument('--keep',action='store_true',help='retain completed tasks for UI inspection')
    args=p.parse_args();client=Client(args.url)
    file=client.upload(pathlib.Path(__file__).resolve().parents[1]/'tests/fixtures/native-p1s.3mf')
    created=client.job({'kind':'PACK','fileId':file['fileId'],'profileId':file['profileId'],'attempts':50,'resolution':0.5})
    print('Created pack',created['jobId'],flush=True)
    packed=client.wait(created['jobId'],args.timeout);client.verify_downloads(packed)
    clean=client.call('GET',f'/jobs/{created["jobId"]}')
    assert not any(a['name'].endswith(('.json','.txt')) for a in clean['artifacts'])
    assert any(a['name'].endswith('.txt') for a in packed['artifacts'])
    ids=[created['jobId']]
    if not args.pack_only:
        scheduled=client.job({'kind':'SCHEDULE','packJobId':created['jobId'],'profileId':file['profileId']})
        ids.append(scheduled['jobId']);client.verify_downloads(client.wait(scheduled['jobId'],args.timeout))
    if not args.keep:
        for id in reversed(ids):client.call('DELETE',f'/jobs/{id}')
        client.call('DELETE',f'/uploads/{file["fileId"]}')
    print('PASS: guest/CSRF/PoW/upload/queue/native/S3/debug',flush=True)

if __name__=='__main__':main()

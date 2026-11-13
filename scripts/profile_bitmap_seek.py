#!/usr/bin/env python3
"""Profile two correct bitmap seek graphs on original local synthetic media.
This is FFmpeg-only warm-file diagnosis, not LAN/NAS/production acceptance.
"""
import argparse, hashlib, json, os, platform, random, resource, statistics, struct
import subprocess, tempfile, time, selectors
from pathlib import Path

def run(command, timeout=90):
    proc = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    buffers = {'out': bytearray(), 'err': bytearray()}
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(proc.stdout, selectors.EVENT_READ, 'out')
            selector.register(proc.stderr, selectors.EVENT_READ, 'err')
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0: raise TimeoutError('FFmpeg diagnostic deadline')
                for key, _ in selector.select(min(remaining, 0.1)):
                    chunk = os.read(key.fd, 65536)
                    if not chunk: selector.unregister(key.fileobj);continue
                    if len(buffers[key.data]) + len(chunk) > 1 << 20:
                        raise RuntimeError('diagnostic output limit exceeded')
                    buffers[key.data].extend(chunk)
        code = proc.wait(timeout=max(0.001, deadline-time.monotonic()))
        if code: raise RuntimeError(buffers['err'][-8192:].decode(errors='replace'))
        return bytes(buffers['out'])
    finally:
        if proc.poll() is None: proc.kill()
        proc.wait()
        proc.stdout.close();proc.stderr.close()

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--ffmpeg', required=True)
    p.add_argument('--output', required=True)
    p.add_argument('--repetitions', type=int, default=3)
    args = p.parse_args()
    if not 1 <= args.repetitions <= 10:
        p.error('repetitions must be 1..10')
    ff = str(Path(args.ffmpeg).resolve())
    hardware = {'logical_cpus': os.cpu_count()}
    if platform.system() == 'Darwin':
        fields = run(['/usr/sbin/sysctl','-n','machdep.cpu.brand_string','hw.model','hw.memsize']).decode().splitlines()
        hardware.update(cpu_model=fields[0],model=fields[1],ram_bytes=int(fields[2]))
    elif Path('/proc/cpuinfo').exists():
        hardware['cpu_model'] = next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')), 'unknown')
        hardware['ram_bytes'] = os.sysconf('SC_PHYS_PAGES')*os.sysconf('SC_PAGE_SIZE')
    with tempfile.TemporaryDirectory(prefix='ovl-bitmap-profile-') as folder:
        root = Path(folder)
        sup = bytearray()
        def packet(second, kind, body):
            sup.extend(b'PG' + struct.pack('>IIBH', second * 90000, 0, kind, len(body)) + body)
        def pcs(second, visible):
            body = struct.pack('>HHBHBBBB',1280,720,0x10,int(not visible),0x80 if visible else 0,0,0,int(visible))
            if visible: body += struct.pack('>HBBHH',0,0,0,200,560)
            packet(second,0x16,body)
        pcs(1,True)
        packet(1,0x17,struct.pack('>BBHHHH',1,0,200,560,160,40))
        packet(1,0x14,bytes([0,0,0,16,128,128,0,1,235,128,128,255]))
        rle = (bytes([1])*160+b'\0\0')*40
        packet(1,0x15,b'\0\0\0\xc0'+(len(rle)+4).to_bytes(3,'big')+struct.pack('>HH',160,40)+rle)
        packet(1,0x80,b'');pcs(63,False);packet(63,0x80,b'')
        (root/'cue.sup').write_bytes(sup)
        source = root/'source.mkv'
        run([ff,'-nostdin','-v','error','-f','lavfi','-i','color=black:size=1280x720:rate=12','-f','lavfi','-i','sine=frequency=440','-itsoffset','1','-i',str(root/'cue.sup'),'-map','0:v','-map','1:a','-map','2:s','-t','64','-c:v','libx264','-preset','ultrafast','-threads','2','-g','12','-c:a','aac','-c:s','copy','-fs','8388608',str(source)])
        if source.stat().st_size > 8 << 20: raise RuntimeError('fixture exceeds 8MiB budget')
        def graph(name):
            if name == 'decode_video_preroll':
                return ['-i',str(source),'-filter_complex','[0:v][0:s:0]overlay=eof_action=pass:repeatlast=0[v]']
            return ['-ss','60','-i',str(source),'-i',str(source),'-filter_complex','[1:s:0]setpts=PTS-60/TB[s];[0:v][s]overlay=eof_action=pass:repeatlast=0[v]']
        names=['decode_video_preroll','subtitle_only_preroll']
        quality=[]
        for name in names:
            for relative, visible in [(0.25,True),(3.5,False)]:
                seek=60+relative if name==names[0] else relative
                pixels=run([ff,'-nostdin','-v','error',*graph(name),'-map','[v]','-ss',str(seek),'-frames:v','1','-pix_fmt','gray','-f','rawvideo','pipe:1'])
                if len(pixels)!=1280*720: raise RuntimeError('incomplete decoded picture')
                value=pixels[580*1280+280]
                if not (value>220 if visible else value<20): raise RuntimeError(f'{name}: incorrect cue pixel {value}')
                quality.append({'strategy':name,'relative_seconds':relative,'visible':visible,'pixel':value})
        trials=[];order=names*args.repetitions;random.Random(42).shuffle(order)
        for name in order:
            command=[ff,'-nostdin','-v','error',*graph(name),'-map','[v]','-map','0:a:0']
            if name==names[0]:command += ['-ss','60']
            command += ['-t','2','-c:v','libx264','-preset','veryfast','-threads','2','-filter_complex_threads','2','-c:a','aac','-f','null','-']
            before=resource.getrusage(resource.RUSAGE_CHILDREN);start=time.monotonic();run(command)
            after=resource.getrusage(resource.RUSAGE_CHILDREN)
            trials.append({'strategy':name,'wall_seconds':time.monotonic()-start,'ffmpeg_cpu_seconds':after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime,'errors':0})
        peak=resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        report={'script_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'scope':'Warm-file FFmpeg-only 64s black PGS fixture, seek60s/encode2s. Synthetic diagnosis, not LAN/SMB/production acceptance or a language comparison. Child peak RSS includes fixture generation and quality runs; it is not per-strategy RAM.','host':platform.platform(),'hardware':hardware,'ffmpeg_sha256':hashlib.sha256(Path(ff).read_bytes()).hexdigest(),'ffmpeg_version':run([ff,'-version']).decode().splitlines()[:3],'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'source_bytes':source.stat().st_size,'child_peak_rss_mib_including_setup':peak/(1048576 if platform.system()=='Darwin' else 1024),'quality':quality,'trials':trials,'summary':{name:{key:{'median':statistics.median(values),'min':min(values),'max':max(values),'stdev':statistics.stdev(values) if len(values)>1 else 0} for key in ['wall_seconds','ffmpeg_cpu_seconds'] for values in [[t[key] for t in trials if t['strategy']==name]]} for name in names}}
        Path(args.output).parent.mkdir(parents=True,exist_ok=True);Path(args.output).write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps(report['summary'],indent=2))
if __name__=='__main__':main()

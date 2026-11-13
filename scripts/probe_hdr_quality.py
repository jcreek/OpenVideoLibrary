#!/usr/bin/env python3
"""Bounded VAAPI graph diagnostic with actual PQ grayscale; not acceptance."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def pq(nits):
    power = (nits / 10000) ** (2610 / 16384)
    return ((3424/4096 + 2413/128*power)/(1 + 2392/128*power)) ** (2523/32)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True)
    args=parser.parse_args()
    report={'scope':'Calibrated PQ grayscale graph diagnostic, no production/visual-quality acceptance','graphs':[]}
    def run(command):
        result=subprocess.run(command,capture_output=True,timeout=30)
        return result
    with tempfile.TemporaryDirectory(prefix='ovl-hdr-quality-') as tmp:
        root=Path(tmp)
        raw=root/'signal.yuv'
        values=[round(64+876*pq(n)) for n in [0,10,100,1000]]
        y=b''.join(values[x//80].to_bytes(2,'little') for _ in range(180) for x in range(320))
        frame=y+(512).to_bytes(2,'little')*(320*180//2)
        raw.write_bytes(frame*36)
        source=root/'source.mp4'
        command=['ffmpeg','-nostdin','-v','error','-f','rawvideo','-pixel_format','yuv420p10le','-video_size','320x180','-framerate','12','-i',str(raw),'-c:v','libx265','-preset','ultrafast','-threads','2','-x265-params','pools=1:frame-threads=1:log-level=error:hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400','-color_primaries','bt2020','-color_trc','smpte2084','-colorspace','bt2020nc','-color_range','tv',str(source)]
        result=run(command)
        if result.returncode: raise RuntimeError(result.stderr.decode(errors='replace')[-8192:])
        report['fixture_sha256']=hashlib.sha256(source.read_bytes()).hexdigest()
        tone='tonemap_vaapi=format=nv12:primaries=bt709:transfer=bt709:matrix=bt709'
        scale='scale_vaapi=w=320:h=180:format=nv12'
        cases=[('hw-tone-scale',True,tone+','+scale),('hw-scale-tone',True,'scale_vaapi=w=320:h=180:format=p010,'+tone),('sw-tone-scale',False,'format=p010le,hwupload,'+tone+','+scale),('hw-tone',True,tone),('sw-tone',False,'format=p010le,hwupload,'+tone),('hw-tone-p010-scale',True,tone.replace('format=nv12','format=p010')+','+scale),('hw-setparams-tone',True,'setparams=range=limited:color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,'+tone)]
        for name,hw,filters in cases:
            output=root/'output.mp4'
            command=['ffmpeg','-nostdin','-v','error','-y','-vaapi_device','/dev/dri/renderD128']
            if hw: command+=['-hwaccel','vaapi','-hwaccel_output_format','vaapi']
            command+=['-ss','1.25','-i',str(source),'-vf',filters,'-an','-c:v','h264_vaapi','-b:v','1M','-color_primaries','bt709','-color_trc','bt709','-colorspace','bt709',str(output)]
            result=run(command)
            row={'name':name,'command':command,'exit_code':result.returncode,'stderr':result.stderr.decode(errors='replace')[-8192:]}
            if result.returncode==0:
                decoded=run(['ffmpeg','-nostdin','-v','error','-i',str(output),'-frames:v','1','-pix_fmt','rgb24','-f','rawvideo','pipe:1'])
                row['decode_exit_code']=decoded.returncode
                if len(decoded.stdout)==320*180*3:
                    row['patch_rgb']=[list(decoded.stdout[(90*320+x)*3:(90*320+x)*3+3]) for x in [40,120,200,280]]
                row['output_sha256']=hashlib.sha256(output.read_bytes()).hexdigest()
            report['graphs'].append(row)
            print(name,row.get('patch_rgb'),row['exit_code'],flush=True)
    Path(args.output).write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':main()

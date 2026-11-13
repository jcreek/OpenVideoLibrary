#!/usr/bin/env python3
"""Bounded physical VAAPI HDR feasibility probe using generated, tagged HDR10.

This is not visual-quality or HDR release acceptance. It does not enable hardware
HDR in the server. Fixture and output storage live in one disposable directory.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True)
    args=parser.parse_args()
    report={'recorded_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'scope':'VAAPI HDR graph feasibility only; tagged synthetic pattern; no visual-quality acceptance',
            'hardware_hdr_enabled_in_server':False,'checks':[],'supported':False}
    def run(name,command):
        start=time.monotonic()
        try:
            result=subprocess.run(command,capture_output=True,text=True,timeout=30)
            row={'name':name,'exit_code':result.returncode,'seconds':time.monotonic()-start,
                 'stdout':result.stdout[-8192:],'stderr':result.stderr[-8192:]}
        except subprocess.TimeoutExpired:
            row={'name':name,'exit_code':None,'seconds':time.monotonic()-start,'error':'30 second deadline exceeded'}
        report['checks'].append(row)
        return row['exit_code']==0
    with tempfile.TemporaryDirectory(prefix='ovl-hdr-probe-') as tmp:
        root=Path(tmp)
        source=root/'source.mp4'
        fixture=['ffmpeg','-nostdin','-hide_banner','-v','error','-f','lavfi','-i','testsrc2=size=320x180:rate=12','-t','1','-c:v','libx265','-preset','ultrafast','-threads','2','-x265-params','pools=1:frame-threads=1:log-level=error:hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400','-pix_fmt','yuv420p10le','-color_primaries','bt2020','-color_trc','smpte2084','-colorspace','bt2020nc',str(source)]
        if run('hdr10-fixture',fixture):
            report['fixture_sha256']=hashlib.sha256(source.read_bytes()).hexdigest()
            output=root/'sdr.mp4'
            command=['ffmpeg','-nostdin','-hide_banner','-v','error','-vaapi_device','/dev/dri/renderD128','-hwaccel','vaapi','-hwaccel_output_format','vaapi','-i',str(source),'-vf','tonemap_vaapi=format=nv12:primaries=bt709:transfer=bt709:matrix=bt709,scale_vaapi=w=320:h=180:format=nv12','-c:v','h264_vaapi','-b:v','1M','-color_primaries','bt709','-color_trc','bt709','-colorspace','bt709',str(output)]
            if run('physical-vaapi-hdr',command):
                decoded=run('decode-sdr-output',['ffmpeg','-nostdin','-hide_banner','-v','error','-i',str(output),'-f','null','-'])
                report['supported']=decoded
                report['output_sha256']=hashlib.sha256(output.read_bytes()).hexdigest()
    Path(args.output).parent.mkdir(parents=True,exist_ok=True)
    Path(args.output).write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))
    if not report['supported']:
        raise SystemExit(1)


if __name__=='__main__':
    main()

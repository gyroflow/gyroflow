# SPDX-License-Identifier: GPL-3.0-or-later
"""Stream a labeled local comparison of the official real-footage test renders."""
from pathlib import Path
import argparse,json,subprocess,hashlib
import numpy as np
from PIL import Image,ImageDraw,ImageFont

parser=argparse.ArgumentParser()
parser.add_argument('--ffmpeg',required=True)
parser.add_argument('--directory',required=True)
parser.add_argument('--video',action='append',help='label=filename; exactly three inputs')
parser.add_argument('--frames',type=int,default=870)
parser.add_argument('--fps',default='60000/1001')
parser.add_argument('--output',default='real-footage-comparison.mp4')
args=parser.parse_args()
root=Path(args.directory).resolve()
paths=[root/'video-only.mp4',root/'reserved-control.mp4',root/'reserved-residual.mp4']
labels=['Input without gyro','Camera only (matched crop)','Residual correction']
if args.video:
    entries=[v.split('=',1) for v in args.video]
    if len(entries)!=3:raise SystemExit('Exactly three inputs are required')
    labels=[v[0] for v in entries];paths=[root/v[1] for v in entries]
w,h,frames=640,360,args.frames
video=root/args.output
if video.exists(): raise SystemExit('Comparison already exists; inspect before replacing')
header=Image.new('RGB',(w*3,32),'black');draw=ImageDraw.Draw(header)
try:
    font=ImageFont.truetype('DejaVuSans.ttf',18)
except OSError:
    try:
        font=ImageFont.truetype('C:/Windows/Fonts/arial.ttf',18)
    except OSError:
        font=ImageFont.load_default(size=18)
for i,label in enumerate(labels): draw.text((i*w+10,5),label,font=font,fill='white')
header=np.asarray(header)
logs=[];decoders=[]
command=[args.ffmpeg,'-v','error','-n','-f','rawvideo','-pix_fmt','rgb24','-s',f'{w*3}x{h+32}',
    '-r',args.fps,'-i','pipe:0','-an','-c:v','libx264','-threads','8','-preset','fast','-crf','20','-pix_fmt','yuv420p',str(video)]
with (root/'demo-encode.log').open('wb') as encode_log:
    encoder=subprocess.Popen(command,stdin=subprocess.PIPE,stderr=encode_log)
    try:
        for i,path in enumerate(paths):
            log=(root/f'demo-decode-{i}.log').open('wb');logs.append(log)
            decoders.append(subprocess.Popen([args.ffmpeg,'-v','error','-i',str(path),'-vf',f'scale={w}:{h}',
                '-pix_fmt','rgb24','-f','rawvideo','pipe:1'],stdout=subprocess.PIPE,stderr=log))
        for index in range(frames):
            panels=[]
            for decoder in decoders:
                raw=decoder.stdout.read(w*h*3)
                if len(raw)!=w*h*3: raise ValueError(f'Missing frame {index}')
                panels.append(np.frombuffer(raw,dtype=np.uint8).reshape(h,w,3))
            frame=np.concatenate([header,np.concatenate(panels,axis=1)],axis=0)
            encoder.stdin.write(frame.tobytes())
            if index==min(240,frames//2): Image.fromarray(frame).save(root/(video.stem+'-frame.png'))
    finally:
        encoder.stdin.close()
        for decoder in decoders:
            decoder.stdout.close()
            decoder.wait(timeout=30)
        for log in logs: log.close()
    if encoder.wait(timeout=60): raise SystemExit('Encoding failed')
(root/'demo.json').write_text(json.dumps({'video':str(video),'frames':frames,'command':command,
    'sha256':hashlib.sha256(video.read_bytes()).hexdigest(),
    'scope':'Labeled comparison of the supplied input, camera-only and residual-corrected clips.'},indent=2)+'\n')
print(json.dumps({'video':str(video),'frames':frames,'bytes':video.stat().st_size}))

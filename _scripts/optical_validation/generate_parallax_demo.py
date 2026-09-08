# SPDX-License-Identifier: GPL-3.0-or-later
"""Original perspective scene: camera jitter, deliberate pan, depth and moving foreground."""
from pathlib import Path
import argparse,hashlib,json,math,subprocess
import numpy as np
from PIL import Image,ImageDraw

parser=argparse.ArgumentParser()
parser.add_argument('--ffmpeg',required=True)
parser.add_argument('--output',required=True)
args=parser.parse_args();root=Path(args.output).resolve();root.mkdir(parents=True,exist_ok=False)
w,h,fps,count,up=640,360,30,180,2
focal=480.0

def color(x,y,seed):
    v=(x*73856093)^(y*19349663)^(seed*83492791);v^=v>>13;v=(v*1274126177)&0xffffffff
    shade=70+(v>>24)//2
    return (shade,min(230,shade+20),min(240,shade+30))

polygons=[]
# Floor and ceiling tile patterns, with nonrepeating tones for optical correspondence.
for z in range(3,25):
    for x in range(-10,10):
        for y,seed in [(-1.7,1),(3.5,2)]:
            points=np.array([[x,y,z],[x+1,y,z],[x+1,y,z+1],[x,y,z+1]],float)
            polygons.append((points,color(x,z,seed)))
# Back wall and side walls supply depth and straight-line references.
for x in range(-10,10):
    for y in range(-2,5):
        polygons.append((np.array([[x,y,24],[x+1,y,24],[x+1,y+1,24],[x,y+1,24]],float),color(x,y,3)))
for z in range(3,24):
    for y in range(-2,4):
        for x,seed in [(-10,4),(10,5)]:
            polygons.append((np.array([[x,y,z],[x,y+1,z],[x,y+1,z+1],[x,y,z+1]],float),color(y,z,seed)))

def box(center,extent,tint):
    c=np.asarray(center);e=np.asarray(extent)/2
    corners=np.array([c+e*np.array([x,y,z]) for x,y,z in [(-1,-1,-1),(1,-1,-1),(1,1,-1),(-1,1,-1),(-1,-1,1),(1,-1,1),(1,1,1),(-1,1,1)]])
    faces=[[0,1,2,3],[4,7,6,5],[0,4,5,1],[3,2,6,7],[0,3,7,4],[1,5,6,2]]
    return [(corners[face],tuple(int(v*(0.6+0.07*i)) for v in tint)) for i,face in enumerate(faces)]

polygons+=box([-2,-0.7,9],[1.5,2,1.5],[220,180,70])
polygons+=box([2,-0.9,15],[2,1.6,2],[70,180,230])
video=root/'parallax-input.mp4'
command=[args.ffmpeg,'-v','error','-n','-f','rawvideo','-pixel_format','rgb24','-video_size',f'{w}x{h}','-framerate',str(fps),
    '-i','pipe:0','-an','-c:v','libx264','-threads','8','-preset','fast','-crf','16','-pix_fmt','yuv420p',str(video)]
poses=[]
with (root/'encode.log').open('wb') as log:
    encoder=subprocess.Popen(command,stdin=subprocess.PIPE,stderr=log)
    try:
        for index in range(count):
            t=index/fps
            yaw=0.008*t+0.009*math.sin(2*math.pi*t*4.1)
            pitch=0.006*math.sin(2*math.pi*t*5.3)
            cy,sy,cp,sp=math.cos(yaw),math.sin(yaw),math.cos(pitch),math.sin(pitch)
            rotation=np.array([[cy,0,sy],[0,1,0],[-sy,0,cy]])@np.array([[1,0,0],[0,cp,-sp],[0,sp,cp]])
            camera=np.array([0.04*t+0.06*math.sin(2*math.pi*t*3.7),0.04*math.sin(2*math.pi*t*4.7),0.05*t])
            current=polygons+box([1.5*math.sin(t*0.65),-0.8,5.0],[0.8,1.0,0.7],[230,80,55])
            projected=[]
            for points,shade in current:
                p=(points-camera)@rotation
                if np.min(p[:,2])<=0.05:continue
                xy=np.column_stack([w/2+focal*p[:,0]/p[:,2],h/2-focal*p[:,1]/p[:,2]])*up
                if np.max(xy[:,0])<0 or np.min(xy[:,0])>=w*up or np.max(xy[:,1])<0 or np.min(xy[:,1])>=h*up:continue
                projected.append((float(np.mean(p[:,2])),xy,shade))
            image=Image.new('RGB',(w*up,h*up),(45,52,62));draw=ImageDraw.Draw(image)
            for depth,xy,shade in sorted(projected,key=lambda p:p[0],reverse=True):
                draw.polygon([tuple(p) for p in xy],fill=shade,outline=(28,34,42),width=up)
            frame=image.resize((w,h),Image.Resampling.LANCZOS)
            encoder.stdin.write(frame.tobytes())
            if index==90:frame.save(root/'scene-frame.png')
            poses.append({'frame':index,'timestamp_ms':t*1000,'camera_position':camera.tolist(),'yaw':yaw,'pitch':pitch})
    finally:encoder.stdin.close()
    if encoder.wait(timeout=60):raise SystemExit('Video encoding failed')
lens={'name':'Original parallax demo perspective','camera_brand':'Synthetic','camera_model':'Original projected geometry','lens_model':'Pinhole','calibrated_by':'Original test fixture','calibrator_version':'1.6.3',
    'calib_dimension':{'w':w,'h':h},'orig_dimension':{'w':w,'h':h},'fps':fps,'global_shutter':True,'distortion_model':'opencv_standard',
    'input_horizontal_stretch':1,'input_vertical_stretch':1,'fisheye_params':{'camera_matrix':[[focal,0,w/2],[0,focal,h/2],[0,0,1]],'distortion_coeffs':[0,0,0,0,0]}}
(root/'lens.json').write_text(json.dumps(lens,indent=2)+'\n')
(root/'source.json').write_text(json.dumps({'frames':count,'fps':fps,'size':[w,h],'camera_poses':poses,'sha256':hashlib.sha256(video.read_bytes()).hexdigest(),
    'scope':'Original synthetic 3D scene and moving foreground, generated solely by this script; no third-party footage or gyro stream.'},indent=2)+'\n')
print(json.dumps({'video':str(video),'frames':count,'bytes':video.stat().st_size}))

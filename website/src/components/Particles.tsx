import { useEffect, useRef } from "react";

/* A mind, drawn in light. A shell of 60,000 points holds a lattice of synapses that fire: pulses start at random
 * points and race across the surface, lighting the points and links they pass. Inside, a galaxy turns faster the
 * closer it is to the core, which flashes with every pulse. Rings carry comets around the outside, dust drifts past,
 * and the cursor excites whatever is under it. The silhouette stays a circle; everything else keeps changing. */

const NOISE = /* glsl */ `
vec3 mod289(vec3 x){return x-floor(x*(1.0/289.0))*289.0;}
vec4 mod289(vec4 x){return x-floor(x*(1.0/289.0))*289.0;}
vec4 permute(vec4 x){return mod289(((x*34.0)+1.0)*x);}
vec4 taylorInvSqrt(vec4 r){return 1.79284291400159-0.85373472095314*r;}
float snoise(vec3 v){
  const vec2 C=vec2(1.0/6.0,1.0/3.0); const vec4 D=vec4(0.0,0.5,1.0,2.0);
  vec3 i=floor(v+dot(v,C.yyy)); vec3 x0=v-i+dot(i,C.xxx);
  vec3 g=step(x0.yzx,x0.xyz); vec3 l=1.0-g; vec3 i1=min(g.xyz,l.zxy); vec3 i2=max(g.xyz,l.zxy);
  vec3 x1=x0-i1+C.xxx; vec3 x2=x0-i2+C.yyy; vec3 x3=x0-D.yyy;
  i=mod289(i);
  vec4 p=permute(permute(permute(i.z+vec4(0.0,i1.z,i2.z,1.0))+i.y+vec4(0.0,i1.y,i2.y,1.0))+i.x+vec4(0.0,i1.x,i2.x,1.0));
  float n_=0.142857142857; vec3 ns=n_*D.wyz-D.xzx;
  vec4 j=p-49.0*floor(p*ns.z*ns.z); vec4 x_=floor(j*ns.z); vec4 y_=floor(j-7.0*x_);
  vec4 x=x_*ns.x+ns.yyyy; vec4 y=y_*ns.x+ns.yyyy; vec4 h=1.0-abs(x)-abs(y);
  vec4 b0=vec4(x.xy,y.xy); vec4 b1=vec4(x.zw,y.zw);
  vec4 s0=floor(b0)*2.0+1.0; vec4 s1=floor(b1)*2.0+1.0; vec4 sh=-step(h,vec4(0.0));
  vec4 a0=b0.xzyw+s0.xzyw*sh.xxyy; vec4 a1=b1.xzyw+s1.xzyw*sh.zzww;
  vec3 p0=vec3(a0.xy,h.x); vec3 p1=vec3(a0.zw,h.y); vec3 p2=vec3(a1.xy,h.z); vec3 p3=vec3(a1.zw,h.w);
  vec4 norm=taylorInvSqrt(vec4(dot(p0,p0),dot(p1,p1),dot(p2,p2),dot(p3,p3)));
  p0*=norm.x; p1*=norm.y; p2*=norm.z; p3*=norm.w;
  vec4 m=max(0.6-vec4(dot(x0,x0),dot(x1,x1),dot(x2,x2),dot(x3,x3)),0.0); m=m*m;
  return 42.0*dot(m*m,vec4(dot(p0,x0),dot(p1,x1),dot(p2,x2),dot(p3,x3)));
}`;

/** Shared by every layer: time, the six live pulses, the cursor, and how a pulse lights a point on the shell. */
const COMMON = /* glsl */ `
uniform float uTime; uniform float uPx; uniform float uCam; uniform float uAspect; uniform vec2 uMouse;
uniform vec4 uPulse[6];
${NOISE}
float pulseAt(vec3 p){
  float e=0.0;
  for(int i=0;i<6;i++){
    float age=uTime-uPulse[i].w;
    if(age<0.0||age>4.5) continue;
    float th=acos(clamp(dot(normalize(p),uPulse[i].xyz),-1.0,1.0));
    float w=0.055+age*0.022;
    float d=(th-age*0.85)/w;
    e+=exp(-d*d)*(1.0-age/4.5);
  }
  return e;
}
float flash(){
  float f=0.0;
  for(int i=0;i<6;i++){ float age=uTime-uPulse[i].w; if(age>=0.0) f=max(f,exp(-age*2.2)); }
  return f;
}
vec3 shell(vec3 p,out float e){
  vec3 q=normalize(p);
  float n=snoise(q*1.4+vec3(0.0,uTime*0.10,0.0));
  float n2=snoise(q*3.7-vec3(uTime*0.17,0.0,0.0));
  e=pulseAt(q);
  return q*(1.0+0.010*n+0.004*n2+0.006*e);
}
float cursor(vec4 clip){
  vec2 d=(clip.xy/clip.w-uMouse)*vec2(uAspect,1.0);
  return exp(-dot(d,d)*26.0);
}`;

const SHELL_VS = /* glsl */ `
${COMMON}
attribute float aSeed; varying float vE; varying float vSeed; varying float vDepth;
void main(){
  float e; vec3 pos=shell(position,e);
  vec4 mv=modelViewMatrix*vec4(pos,1.0); vec4 clip=projectionMatrix*mv;
  float m=cursor(clip);
  vE=e+m*0.9; vSeed=aSeed;
  vDepth=clamp((uCam+mv.z)*0.5+0.5,0.0,1.0);
  gl_Position=clip;
  gl_PointSize=(1.3+2.8*vE+aSeed*0.7)*uPx*(5.0/-mv.z);
}`;
const SHELL_FS = /* glsl */ `
precision highp float; varying float vE; varying float vSeed; varying float vDepth;
void main(){
  float d=length(gl_PointCoord-0.5); float a=exp(-d*d*16.0);
  vec3 col=mix(vec3(0.35,0.82,1.0),vec3(0.9,0.98,1.0),clamp(vE,0.0,1.0));
  if(vSeed>0.965) col=mix(vec3(1.0,0.70,0.33),vec3(1.0,0.93,0.8),clamp(vE,0.0,1.0));
  gl_FragColor=vec4(col,a*(0.12+0.88*vDepth)*(0.42+1.2*vE));
}`;

const LINK_VS = /* glsl */ `
${COMMON}
varying float vE; varying float vDepth;
void main(){
  float e; vec3 pos=shell(position*1.0,e);
  vec4 mv=modelViewMatrix*vec4(pos,1.0); vec4 clip=projectionMatrix*mv;
  vE=e+cursor(clip)*0.6; vDepth=clamp((uCam+mv.z)*0.5+0.5,0.0,1.0);
  gl_Position=clip;
}`;
const LINK_FS = /* glsl */ `
precision highp float; varying float vE; varying float vDepth;
void main(){
  vec3 col=mix(vec3(0.30,0.74,1.0),vec3(0.92,0.98,1.0),clamp(vE,0.0,1.0));
  gl_FragColor=vec4(col,(0.07+1.1*vE)*(0.25+0.75*vDepth));
}`;

const CORE_VS = /* glsl */ `
${COMMON}
attribute float aR; attribute float aSeed; varying float vC; varying float vS;
void main(){
  float r=aR; float ang=uTime*(0.55/(0.22+r))*0.6+aSeed*6.2832;
  vec3 p=position; float c=cos(ang), s=sin(ang);
  p=vec3(p.x*c-p.z*s,p.y*0.55,p.x*s+p.z*c);
  vec4 mv=modelViewMatrix*vec4(p,1.0);
  vC=1.0-r; vS=aSeed;
  gl_Position=projectionMatrix*mv;
  gl_PointSize=(1.1+1.6*vC+2.0*flash()*vC)*uPx*(5.0/-mv.z);
}`;
const CORE_FS = /* glsl */ `
precision highp float; varying float vC; varying float vS;
void main(){
  float d=length(gl_PointCoord-0.5); float a=exp(-d*d*14.0);
  vec3 col=mix(vec3(0.35,0.82,1.0),vec3(1.0,0.74,0.38),smoothstep(0.35,0.95,vC));
  gl_FragColor=vec4(col,a*(0.25+0.6*vC));
}`;

const GLOW_VS = /* glsl */ `
${COMMON}
void main(){
  vec4 mv=modelViewMatrix*vec4(0.0,0.0,0.0,1.0);
  gl_Position=projectionMatrix*mv;
  gl_PointSize=(520.0+260.0*flash())*uPx*(5.0/-mv.z);
}`;
const GLOW_FS = /* glsl */ `
precision highp float; uniform float uTime; uniform vec4 uPulse[6];
void main(){
  float f=0.0; for(int i=0;i<6;i++){ float age=uTime-uPulse[i].w; if(age>=0.0) f=max(f,exp(-age*2.2)); }
  float d=length(gl_PointCoord-0.5)*2.0; float a=pow(max(1.0-d,0.0),2.6);
  gl_FragColor=vec4(mix(vec3(1.0,0.72,0.38),vec3(0.55,0.88,1.0),d),a*(0.10+0.16*f));
}`;

const RING_VS = /* glsl */ `
${COMMON}
attribute float aA; attribute float aRing; varying float vH;
void main(){
  float speed=0.28+aRing*0.11; float dir=mod(aRing,2.0)<1.0?1.0:-1.0;
  float a=aA+uTime*speed*dir;
  float R=1.34+aRing*0.17;
  vec3 p=vec3(cos(a)*R,0.0,sin(a)*R);
  float tilt=0.5+aRing*0.55; float c=cos(tilt), s=sin(tilt);
  p=vec3(p.x,p.y*c-p.z*s,p.y*s+p.z*c);
  float roll=aRing*1.3; float cr=cos(roll), sr=sin(roll);
  p=vec3(p.x*cr-p.y*sr,p.x*sr+p.y*cr,p.z);
  // The comet is a brightness ramp over the ring's own angle, so it rides the ring as the points move.
  float head=dir>0.0?fract(aA/6.2832):fract(1.0-aA/6.2832);
  vH=pow(head,9.0);
  vec4 mv=modelViewMatrix*vec4(p,1.0);
  gl_Position=projectionMatrix*mv;
  gl_PointSize=(1.1+3.4*vH)*uPx*(5.0/-mv.z);
}`;
const RING_FS = /* glsl */ `
precision highp float; varying float vH;
void main(){
  float d=length(gl_PointCoord-0.5); float a=exp(-d*d*16.0);
  gl_FragColor=vec4(mix(vec3(0.48,0.66,0.8),vec3(1.0,0.86,0.6),vH),a*(0.34+0.66*vH));
}`;

const DUST_VS = /* glsl */ `
${COMMON}
attribute float aSeed; varying float vA;
void main(){
  float ang=uTime*(0.03+aSeed*0.05); float c=cos(ang), s=sin(ang);
  vec3 p=vec3(position.x*c-position.z*s,position.y+sin(uTime*0.2+aSeed*30.0)*0.03,position.x*s+position.z*c);
  vec4 mv=modelViewMatrix*vec4(p,1.0); vec4 clip=projectionMatrix*mv;
  vA=0.1+0.3*cursor(clip)+0.12*sin(uTime*0.8+aSeed*50.0);
  gl_Position=clip;
  gl_PointSize=(1.0+aSeed*1.4)*uPx*(5.0/-mv.z);
}`;
const DUST_FS = /* glsl */ `
precision highp float; varying float vA;
void main(){
  float d=length(gl_PointCoord-0.5); float a=exp(-d*d*16.0);
  gl_FragColor=vec4(0.55,0.74,0.9,a*max(vA,0.02));
}`;

export default function Particles({ className }: { className?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current!;
    const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
    let stop = () => {};
    let dead = false;

    void import("three").then((THREE) => {
      if (dead) return;
      let renderer: InstanceType<typeof THREE.WebGLRenderer>;
      try {
        renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: false, powerPreference: "high-performance" });
      } catch {
        return; // No WebGL: the page still has its words.
      }
      const small = innerWidth < 760;
      const SHELL = small ? 32000 : 60000;
      const px = Math.min(devicePixelRatio || 1, 2);
      renderer.setPixelRatio(px);
      renderer.setClearColor(0x000000, 0);

      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(40, 1, 0.1, 50);
      const pulses = Array.from({ length: 6 }, () => new THREE.Vector4(0, 1, 0, -100));
      const uniforms = {
        uTime: { value: 0 },
        uPx: { value: px },
        uCam: { value: 5.2 },
        uAspect: { value: 1 },
        uMouse: { value: new THREE.Vector2(9, 9) },
        uPulse: { value: pulses },
      };
      const mat = (vertexShader: string, fragmentShader: string) =>
        new THREE.ShaderMaterial({ uniforms, vertexShader, fragmentShader, transparent: true, depthWrite: false, depthTest: false, blending: THREE.AdditiveBlending });

      const rand = Math.random;
      const unit = () => {
        const z = rand() * 2 - 1, a = rand() * 6.2832, r = Math.sqrt(1 - z * z);
        return [r * Math.cos(a), z, r * Math.sin(a)];
      };
      const geo = (attrs: Record<string, [Float32Array, number]>) => {
        const g = new THREE.BufferGeometry();
        for (const [k, [arr, n]] of Object.entries(attrs)) g.setAttribute(k, new THREE.BufferAttribute(arr, n));
        return g;
      };

      const group = new THREE.Group();
      scene.add(group);

      // Shell: points spread evenly over a sphere.
      const sp = new Float32Array(SHELL * 3), ss = new Float32Array(SHELL);
      for (let i = 0; i < SHELL; i++) {
        sp.set(unit(), i * 3); // random, not a spiral, so there is no moiré
        ss[i] = rand();
      }
      group.add(new THREE.Points(geo({ position: [sp, 3], aSeed: [ss, 1] }), mat(SHELL_VS, SHELL_FS)));

      // Synapses: each of 1,400 nodes links to its three nearest neighbours.
      const M = 1400, nodes: number[][] = [];
      for (let i = 0; i < M; i++) {
        const y = 1 - (2 * (i + 0.5)) / M, r = Math.sqrt(1 - y * y), a = i * 2.399963 + (rand() - 0.5) * 0.08;
        nodes.push([Math.cos(a) * r, y + (rand() - 0.5) * 0.02, Math.sin(a) * r]);
      }
      const seg: number[] = [];
      for (let i = 0; i < M; i++) {
        const best: [number, number][] = [];
        for (let j = 0; j < M; j++) {
          if (j === i) continue;
          const d = nodes[i][0] * nodes[j][0] + nodes[i][1] * nodes[j][1] + nodes[i][2] * nodes[j][2];
          if (best.length < 3 || d > best[best.length - 1][0]) {
            best.push([d, j]);
            best.sort((p, q) => q[0] - p[0]);
            best.length = Math.min(best.length, 3);
          }
        }
        for (const [, j] of best) seg.push(...nodes[i], ...nodes[j]);
      }
      group.add(new THREE.LineSegments(geo({ position: [new Float32Array(seg), 3] }), mat(LINK_VS, LINK_FS)));

      // Galaxy inside: differential rotation, so the core outruns the rim.
      const G = small ? 5000 : 9000;
      const gp = new Float32Array(G * 3), gr = new Float32Array(G), gs = new Float32Array(G);
      for (let i = 0; i < G; i++) {
        const r = Math.pow(rand(), 0.7) * 0.82 + 0.03, [x, y, z] = unit();
        gp.set([x * r, y * r, z * r], i * 3);
        gr[i] = r;
        gs[i] = rand();
      }
      group.add(new THREE.Points(geo({ position: [gp, 3], aR: [gr, 1], aSeed: [gs, 1] }), mat(CORE_VS, CORE_FS)));

      // Core glow.
      group.add(new THREE.Points(geo({ position: [new Float32Array(3), 3] }), mat(GLOW_VS, GLOW_FS)));

      // Rings with a comet each.
      const per = 900, RINGS = 3;
      const ra = new Float32Array(per * RINGS), rr = new Float32Array(per * RINGS);
      for (let k = 0; k < RINGS; k++) for (let i = 0; i < per; i++) ((ra[k * per + i] = (i / per) * 6.2832), (rr[k * per + i] = k));
      group.add(new THREE.Points(geo({ position: [new Float32Array(per * RINGS * 3), 3], aA: [ra, 1], aRing: [rr, 1] }), mat(RING_VS, RING_FS)));

      // Dust shell.
      const D = small ? 2500 : 5000;
      const dp = new Float32Array(D * 3), ds = new Float32Array(D);
      for (let i = 0; i < D; i++) {
        const r = 1.5 + rand() * 1.5, [x, y, z] = unit();
        dp.set([x * r, y * r * 0.8, z * r], i * 3);
        ds[i] = rand();
      }
      scene.add(new THREE.Points(geo({ position: [dp, 3], aSeed: [ds, 1] }), mat(DUST_VS, DUST_FS)));

      const size = () => {
        const w = canvas.clientWidth, h = canvas.clientHeight;
        renderer.setSize(w, h, false);
        camera.aspect = w / h;
        // Keep the whole sphere in view on tall, narrow screens.
        const z = w / h < 1 ? 5.2 / Math.max(w / h, 0.62) * 0.8 : 5.2;
        camera.position.set(0, 0, z);
        uniforms.uCam.value = z;
        uniforms.uAspect.value = w / h;
        camera.updateProjectionMatrix();
      };
      size();

      const target = { x: 0, y: 0 }, tilt = { x: 0, y: 0 };
      const move = (e: PointerEvent) => {
        const r = canvas.getBoundingClientRect();
        const nx = ((e.clientX - r.left) / r.width) * 2 - 1, ny = -(((e.clientY - r.top) / r.height) * 2 - 1);
        target.x = nx;
        target.y = ny;
        uniforms.uMouse.value.set(nx, ny);
      };
      const leave = () => uniforms.uMouse.value.set(9, 9);

      let next = 0.4, n = 0;
      const fire = (t: number) => {
        const [x, y, z] = unit();
        pulses[n++ % 6].set(x, y, z, t);
      };

      const t0 = performance.now();
      let raf = 0;
      const draw = (t: number) => {
        uniforms.uTime.value = t;
        if (t >= next) {
          fire(t);
          next = t + 0.55 + rand() * 1.0;
        }
        tilt.x += (target.x - tilt.x) * 0.04;
        tilt.y += (target.y - tilt.y) * 0.04;
        group.rotation.y = t * 0.07 + tilt.x * 0.5;
        group.rotation.x = 0.28 - tilt.y * 0.3;
        renderer.render(scene, camera);
      };
      const loop = () => {
        draw((performance.now() - t0) / 1000);
        raf = requestAnimationFrame(loop);
      };
      addEventListener("resize", size);
      canvas.addEventListener("pointermove", move);
      canvas.addEventListener("pointerleave", leave);
      if (still) {
        fire(6.7);
        fire(6.9);
        draw(7);
      } else loop();

      stop = () => {
        cancelAnimationFrame(raf);
        removeEventListener("resize", size);
        canvas.removeEventListener("pointermove", move);
        canvas.removeEventListener("pointerleave", leave);
        scene.traverse((o) => {
          const m = o as InstanceType<typeof THREE.Mesh>;
          m.geometry?.dispose();
          (m.material as InstanceType<typeof THREE.Material> | undefined)?.dispose();
        });
        renderer.dispose();
      };
    });

    return () => {
      dead = true;
      stop();
    };
  }, []);

  return <canvas ref={ref} className={className} aria-hidden="true" />;
}

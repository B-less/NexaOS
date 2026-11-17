# Differential fuzz of std/json.nx against Python's json module.
# Build the reader first:  nexac build tests/json/roundtrip.nx -o /tmp/jrt
# then run:  python3 tests/json/diff_fuzz.py SEED COUNT

import json, random, subprocess, sys, math
random.seed(int(sys.argv[1]))
def rand_val(d=0):
    r=random.random()
    if d>4 or r<0.3:
        c=random.randint(0,6)
        if c==0: return None
        if c==1: return random.choice([True,False])
        if c==2: return random.randint(-10**15,10**15)
        if c==3: return random.choice([0.5,-2.25,1e-7,3.14159,1e300,-1e-300,123456.789, 0.1])
        if c==4: return random.randint(-100,100)
        return ''.join(random.choice(['a','é','"','\\','\n','\t','😀','\x01','/',' ','中','\u2028']) for _ in range(random.randint(0,8)))
    if r<0.65: return [rand_val(d+1) for _ in range(random.randint(0,4))]
    return {''.join(random.choice('abcé"\\') for _ in range(random.randint(0,3))): rand_val(d+1) for _ in range(random.randint(0,4))}
def mutate(s):
    for _ in range(random.randint(1,3)):
        i=random.randint(0,len(s)); r=random.random()
        if r<0.4: s=s[:i]+random.choice(['"',',',']','}','[','{',':','\\','\\u12','0','-','e','x',' ','\\ud83d'])+s[i:]
        elif r<0.8: s=s[:i]+s[i+random.randint(1,3):]
        else: s=s[:i]+s[i:i+3]*2+s[i+3:]
    return s
docs=[]
for _ in range(int(sys.argv[2])):
    v=rand_val()
    txt=json.dumps(v, ensure_ascii=random.random()<0.5, indent=None if random.random()<0.7 else 1).replace('\n',' ')
    if random.random()<0.4: txt=mutate(txt)
    docs.append(txt.replace('\n',' '))
out=subprocess.run(['/tmp/jrt'],input='\n'.join(docs)+'\n',capture_output=True,text=True)
if out.returncode!=0: print('CRASH',out.stderr[:500]); sys.exit(1)
lines=out.stdout.split('\n')
bad=0
def strict_load(t):
    def no_const(x): raise ValueError('constant')
    v=json.loads(t, parse_constant=no_const)
    return v
def same(a,b):
    if isinstance(a,bool) or isinstance(b,bool): return a is b
    if isinstance(a,(int,float)) and isinstance(b,(int,float)): return float(a)==float(b)
    if isinstance(a,list) and isinstance(b,list): return len(a)==len(b) and all(same(x,y) for x,y in zip(a,b))
    if isinstance(a,dict) and isinstance(b,dict): return a.keys()==b.keys() and all(same(a[k],b[k]) for k in a)
    return a==b
for d,l in zip(docs,lines):
    try:
        exp=strict_load(d); py_ok=True
        if any(c in d for c in []): pass
    except Exception:
        py_ok=False
    # python accepts lone surrogates and huge numbers->inf; treat specially
    nx_ok=not l.startswith('ERR ')
    def has_inf(v):
        if isinstance(v,float): return math.isinf(v)
        if isinstance(v,list): return any(has_inf(x) for x in v)
        if isinstance(v,dict): return any(has_inf(x) for x in v.values())
        return False
    if py_ok and has_inf(exp): py_ok=False
    if py_ok != nx_ok:
        try:
            # lone surrogate escapes: python accepts, NexaLang refuses on purpose
            if '\\ud8' in d.lower() or '\\udc' in d.lower() or '\\udb' in d.lower() or '\\udd' in d.lower() or '\\ude' in d.lower() or '\\udf' in d.lower() or '\\uda' in d.lower() or '\\ud9' in d.lower(): continue
        except Exception: pass
        bad+=1
        if bad<4: print('VALIDITY', py_ok, nx_ok, repr(d)[:200], l[:200])
        continue
    if py_ok:
        try: back=json.loads(l)
        except Exception as e:
            bad+=1; print('BAD OUTPUT', repr(l)[:200]); continue
        if not same(back,exp):
            bad+=1
            if bad<4: print('VALUE', repr(d)[:200], '->', l[:200])
print(f'seed {sys.argv[1]}: {len(docs)} docs, {bad} disagreements')

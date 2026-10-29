"""Independent integer OPB expectations; bits are exact-width, MSB first."""
import bisect
import functools
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
PROFILES = json.loads((ROOT / 'profiles.json').read_text())['profiles']
BANKS = { # count parameter, width, rendering, schedule
 'regular64': ('regular',64,'binary','rr'),
 'staggered64': ('staggered',64,'binary','staggered'),
 'scalar': ('special',1,'scalar','rr'),
 'wide1024': ('special',1024,'binary','rr'),
 'constant64': ('special',64,'constant','constant'),
 'sparse64': ('special',64,'binary','sparse'),
 'dense64': ('special',64,'binary','rr'),
 'mixed_xz64': ('special',64,'mixed','rr'),
 'all_x64': ('special',64,'all-x','constant'),
 'drivers': (8,1,'driver','driver'),
 'control': (1,64,'binary','every'),
 'payload': ('special',1024,'payload','payload'),
 'low_active1024': (1,1024,'low','every'),
 'prior': ('prior',64,'binary','rr'),
 'background': ('background',64,'binary','background'),
}


def config(profile):
    return PROFILES[profile] if isinstance(profile,str) else profile


def inventory(profile):
    p=config(profile)
    for bank,(count,width,kind,schedule) in BANKS.items():
        for i in range(p[count] if isinstance(count,str) else count):
            yield dict(id=f'{bank}/{i}',path=['tb_opb',bank,f'word[{i}]','data','q'],
                       width=width,bank=bank,index=i,kind=kind,schedule=schedule)
    def tree(path,height,node=1,level=0,bank='tree'):
        yield dict(id='/'.join(path[1:]),path=path+['local_value'],width=64,
                   bank=bank,index=node,kind='tree',schedule='constant')
        if level<height:
            yield from tree(path+['branch','left'],height,node*2,level+1,bank)
            yield from tree(path+['branch','right'],height,node*2+1,level+1,bank)
    yield from tree(['tb_opb','tree'],p['tree_height'])
    path=['tb_opb','spine']
    for level in range(p['spine_depth']+1):
        yield dict(id=f'spine/{level}',path=path+['local_value'],width=64,
                   bank='spine',index=level,kind='spine',schedule='constant')
        if level<p['spine_depth']:
            yield dict(id=f'spine-side/{level}',path=path+['branch','side','q'],width=64,
                       bank='spine',index=500000+level,kind='constant',schedule='constant')
            path=path+['branch','child']
    yield dict(id='deep-active',path=path+['endpoint','activity','q'],width=64,
               bank='deep-active',index=700000,kind='binary',schedule='every')
    if p['extra_height']>=0:
        yield from tree(['tb_opb','extra','tree'],p['extra_height'],bank='extra')
    yield dict(id='end_marker',path=['tb_opb','end_marker','q'],width=1,
               bank='end_marker',index=0,kind='marker',schedule='end')


@functools.lru_cache(maxsize=8)
def by_id(profile):
    return {d['id']:d for d in inventory(profile)}


def arithmetic_steps(first,period,steps):
    return range(first,steps+1,period)


def update_steps(p,d):
    """Closed form update positions, independently derived from schedule contract."""
    n=p[BANKS[d['bank']][0]] if d['bank'] in BANKS and isinstance(BANKS[d['bank']][0],str) else 0
    i=d['index']; steps=p['steps']; mode=d['schedule']
    if mode=='constant': return ()
    if mode=='end': return (steps,)
    if mode=='every': return range(1,steps+1)
    if mode=='rr': return arithmetic_steps(i+1,n,steps)
    if mode=='sparse': return arithmetic_steps(100*(i+1),100*n,steps)
    if mode=='driver': return arithmetic_steps(16+i*2,16+i*2,steps)
    if mode=='payload': return range(1,steps+1) if i==0 else arithmetic_steps(i+1,n,steps)
    if mode=='background':
        intensity=p['intensity']
        # An index occurs in the flat stream of intensity consecutive writes per step.
        return sorted({q//intensity+1 for q in range(i,steps*intensity,n)})
    if mode=='staggered':
        if i==0: return arithmetic_steps(1,4,steps)
        hot=min(n-1,99)
        if i<100:
            a=arithmetic_steps(2*i,2*hot,steps)
            if n>100: return a
            b=arithmetic_steps(4*(i-1)+3,4*hot,steps)
            return sorted((*a,*b))
        return arithmetic_steps(4*(i-100)+3,4*(n-100),steps)
    raise ValueError(mode)


MASK=(1<<64)-1

def binary(seed,i,n,width):
    base=(0x9e3779b97f4a7c15*(i+1)+0xd1342543de82ef95*n+seed)&MASK
    return ''.join(f'{base ^ ((0x94d049bb133111eb*(j+1))&MASK):064b}'
                   for j in reversed(range((width+63)//64)))[-width:]


def render(p,d,n,s):
    kind=d['kind']; w=d['width']; i=d['index']
    if kind=='mixed': return ''.join('01xz'[(j+n)%4] for j in reversed(range(w)))
    if kind=='all-x': return 'x'*w
    if kind=='scalar': return str(n%2)
    if kind=='driver': return str((s//(16+i*2))%2)
    if kind=='marker': return str(int(s>0))
    if kind=='tree': return f'{0x100000000+i+p["seed"]:064b}'
    if kind=='spine': return f'{0x200000000+i+p["seed"]:064b}'
    v=binary(p['seed'],i,0 if kind in ('constant','low') else n,w)
    if kind=='payload': v=v[:-1]+str((s//16)%2)
    if kind=='low': v=v[:-1]+str(n%2)
    return v


def expected_events(profile,d):
    p=config(profile); dt=p['t1']//p['steps']
    old=render(p,d,0,0)
    yield 0,old
    for n,s in enumerate(update_steps(p,d),1):
        value=render(p,d,n,s)
        if value!=old:
            yield s*dt,value
            old=value


def value_at(profile,logical_id,raw_tick):
    p=config(profile)
    if raw_tick<0 or raw_tick>p['t1']: raise ValueError('tick outside corpus')
    d=by_id(profile)[logical_id]; steps=update_steps(p,d)
    ordinal=bisect.bisect_right(steps,raw_tick//(p['t1']//p['steps']))
    s=steps[ordinal-1] if ordinal else 0
    return render(p,d,ordinal,s)


def iter_changes(profile,logical_id,start,end):
    """Yield Changes strictly after start, through inclusive end; no Initial."""
    p=config(profile);d=by_id(profile)[logical_id];dt=p['t1']//p['steps']
    steps=update_steps(p,d)
    lo=bisect.bisect_right(steps,start//dt)
    hi=bisect.bisect_right(steps,end//dt)
    old=render(p,d,lo,steps[lo-1] if lo else 0)
    for i in range(lo,hi):
        s=steps[i];bits=render(p,d,i+1,s)
        if bits!=old:yield s*dt,bits
        old=bits

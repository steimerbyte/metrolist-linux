import re, sys
src, dst = sys.argv[1], sys.argv[2]
lines = open(src).read().split('\n')
while lines and (lines[-1].strip()=='' or lines[-1].strip()=='} // verus!'):
    lines.pop()
lines.append('')

ITEM = re.compile(r'^(pub )?(fn|proof fn|open spec fn|spec fn|struct|enum|const|static|impl|type|use|mod)\b|^impl\b')

def item_bounds(lines):
    starts=[i for i,l in enumerate(lines) if ITEM.match(l)]
    bounds=[]
    for k,s in enumerate(starts):
        e = starts[k+1] if k+1 < len(starts) else len(lines)
        bounds.append((s,e))
    return bounds

def strip_invariants(seg):
    out=[]; i=0
    while i < len(seg):
        if re.match(r'^\s*invariant\b', seg[i]):
            while i < len(seg) and not re.match(r'^\s*decreases\s', seg[i]):
                i += 1
            continue
        out.append(seg[i]); i+=1
    return out

def strip_asserts(seg):
    out=[]; i=0
    while i < len(seg):
        if re.match(r'^\s*assert\(', seg[i]):
            d=0
            while i < len(seg):
                d += seg[i].count('(') - seg[i].count(')')
                i += 1
                if d <= 0: break
            continue
        out.append(seg[i]); i+=1
    return out


def strip_proof_blocks(seg):
    out=[]; i=0
    while i < len(seg):
        line=seg[i]
        if re.match(r'^\s*proof\s*\{\s*$', line) or re.match(r'^\s*proof\s*\{.*\}\s*$', line):
            if re.match(r'^\s*proof\s*\{.*\}\s*$', line) and line.count('{')==line.count('}'):
                i += 1
                continue
            d = line.count('{') - line.count('}')
            i += 1
            while i < len(seg) and d > 0:
                d += seg[i].count('{') - seg[i].count('}')
                i += 1
            continue
        out.append(line); i+=1
    return out

out=[]
head_end = item_bounds(lines)[0][0] if item_bounds(lines) else len(lines)
out.extend(lines[:head_end])
for s,e in item_bounds(lines):
    seg = lines[s:e]
    if re.match(r'^(pub )?proof fn ', seg[0]):
        # drop the item and any doc comments that precede it in `out`
        while out and (out[-1].startswith('///') or out[-1].startswith('//!')):
            out.pop()
        while out and out[-1].strip()=='' and out[-1] is not lines[0]:
            out.pop()
        out.append('')
        continue
    seg = strip_invariants(seg)
    seg = strip_asserts(seg)
    seg = strip_proof_blocks(seg)
    out.extend(seg)
# collapse duplicate blank lines before close
while len(out) > 1 and out[-1].strip()=='' and out[-2].strip()=='':
    out.pop()
while out and out[-1].strip()=='':
    out.pop()
out.append('} // verus!')
open(dst,'w').write('\n'.join(out))

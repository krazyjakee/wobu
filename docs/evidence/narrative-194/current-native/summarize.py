import json, statistics
from pathlib import Path
root=Path('/tmp/wobu194-native')
def metric(values):
 if not values:return None
 return {'n':len(values),'min':min(values),'median':statistics.median(values),'p95_inclusive':statistics.quantiles(values,n=100,method='inclusive')[94] if len(values)>1 else values[0],'max':max(values)}
rows=json.loads((root/'read-samples.json').read_text())
groups={}
for row in rows:
 label=row['label'];group=label.rsplit('_',1)[0] if label.rsplit('_',1)[-1].isdigit() else label
 groups.setdefault(group,[]).append(row)
out={}
for name,items in groups.items():
 out[name]={'ms':metric([r['ms'] for r in items]),'frameIntervalsMs':metric([t for r in items for t in r['frameIntervals']]),'perCallMaximumFrameMs':metric([max(r['frameIntervals'],default=0) for r in items]),'outputBytes':metric([r['outputBytes'] for r in items]),'domNodes':metric([r['domNodes'] for r in items])}
mem=json.loads((root/'read-memory.json').read_text())
out['processTreeRssBytes']=metric([r['rssBytes'] for r in mem])
out['notes']=['Inclusive interpolated p95; all samples and outliers retained.', 'Frame intervals include two post-result requestAnimationFrame callbacks; first callback begins at timer installation, not the preceding display frame.', 'Summed native and descendant RSS duplicates shared pages; it is not PSS.', 'No claim of controlled cold disk or cold index.', 'Fewer than 30 samples do not establish p95 compliance.']
(root/'read-summary.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps(out,indent=2))

"""Prepared driver; only run after explicit native/quiet handoff. No mocked IPC."""
import json,time,os,subprocess,hashlib,sys
from pathlib import Path
BASE=Path('/tmp/wobu-acceptance-20260908')
OUT=Path('/tmp/wobu194-native');OUT.mkdir(exist_ok=True)
FIXTURE=Path('/tmp/wobu194-profile/narrative-scale.wobu')
ID=str(732).zfill(26)
QUERY=dict(query='',participant='',quest='',act='',arc='',tag='',policy='',review='',freshness='',missing=False,includeDrafts=False,sort='name',offset=0,limit=25,revision=None)

def send(op,**args):
 command={'id':str(time.time_ns()),'op':op,**args}
 (BASE/'commands'/f"{command['id']}.json").write_text(json.dumps(command))
 result=BASE/'results'/f"{command['id']}.json"
 started=time.monotonic()
 while time.monotonic()-started<180:
  if result.exists():
   data=json.loads(result.read_text())
   if not data['ok']:raise RuntimeError(data)
   return data['value']
  time.sleep(.05)
 raise TimeoutError(command)

def invoke(command,args=None,full=False):
 result=send('profileInvoke',command=command,args=args or {},includeValue=full)
 if result.get('error'):raise RuntimeError(result)
 return result

def canonical():
 files=[FIXTURE/'project.json']
 files += [p for p in (FIXTURE/'narrative').rglob('*') if p.is_file() and 'layout' not in p.relative_to(FIXTURE/'narrative').parts]
 files += list((FIXTURE/'nodes').rglob('*.md'))
 return {str(p.relative_to(FIXTURE)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(files)}

def memory(pid):
 processes={}
 for line in subprocess.check_output(['ps','-eo','pid=,ppid=,rss=,comm='],text=True).splitlines():
  p,parent,rss,name=line.split(None,3);processes[int(p)]={'pid':int(p),'parent':int(parent),'rssKiB':int(rss),'name':name}
 selected={pid}
 while True:
  expanded=selected|{p for p,row in processes.items() if row['parent'] in selected}
  if expanded==selected:break
  selected=expanded
 rows=[processes[p] for p in sorted(selected) if p in processes]
 return {'time':time.time(),'processes':rows,'rssBytes':sum(row['rssKiB']*1024 for row in rows),'scope':'sum RSS of native process and descendants, includes shared-page duplication; not PSS'}

def read_phase(pid):
 records=[];rss=[]
 def record(label,command,args=None,full=False):
  result=invoke(command,args,full);result['label']=label;records.append(result)
  (OUT/'read-samples.json').write_text(json.dumps(records,indent=2)+'\n')
  rss.append(memory(pid));(OUT/'read-memory.json').write_text(json.dumps(rss,indent=2)+'\n')
  print(label,round(result['ms'],1),flush=True);return result
 before=canonical();(OUT/'read-source-before.json').write_text(json.dumps(before,indent=2)+'\n')
 record('project_open','project_open',{'path':str(FIXTURE)})
 first=record('first_browse','narrative_library_query',{'query':QUERY})
 assert first['sceneCount']==1000 and first['rows']==25
 for sample in range(30):
  found=record(f'search_{sample}','narrative_library_query',{'query':{**QUERY,'query':f'ASHFALL-PHRASE-{731+sample:04}'}})
  assert found['total']==1 and found['summaries'][0]['id']==str(732+sample).zfill(26)
  assert found['summaries'][0]['matchCount']==50 and found['summaries'][0]['snippets']<=5
  combined=record(f'combined_{sample}','narrative_library_query',{'query':{**QUERY,'query':'logbook','quest':str(4000011).zfill(26),'policy':'edited'}})
  assert 0<combined['rows']<=25 and combined['total']>0
  assert len({row['id'] for row in combined['summaries']})==combined['rows']
  assert all(str(4000011).zfill(26) in row['quests'] for row in combined['summaries'])
  selected=record(f'scene_{sample}','narrative_scene_get',{'sceneId':ID})
  assert selected['sceneId']==ID
  review=record(f'review_{sample}','narrative_review_get',{'sceneId':ID,'stateJson':None})
  assert review['sceneId']==ID and review['reviewLines']==50
 after=canonical();(OUT/'read-source-after.json').write_text(json.dumps(after,indent=2)+'\n')
 assert before==after,'Read-only measurements changed canonical files'
 print('READ PASS canonical hashes unchanged',flush=True)

def action_phase(pid):
 records=[]
 initial=invoke('narrative_review_get',{'sceneId':ID,'stateJson':None},True)['value']
 current=initial
 for label,action in [('approve',{'kind':'approve'}),('lock',{'kind':'policy','scope':'variant','policy':'locked'}),('unlock',{'kind':'policy','scope':'variant','policy':'edited'})]:
  line=current['lines'][0]
  request={'guard':current['guard'],'target':line['target'],'context_revision':line['context_revision'],'state_json':current['state_json'],'action':action}
  result=invoke('narrative_review_apply',{'request':request},True)
  result['label']=label;current=result['value']['review'];records.append(result)
  (OUT/'action-samples.json').write_text(json.dumps(records,indent=2)+'\n')
  print(label,round(result['ms'],1),max(result['frameIntervals'],default=0),flush=True)
 file=invoke('narrative_scene_get',{'sceneId':ID},True)['value']
 scene=file['scene'];scene['summary']='Native #194 guarded save measurement; synthetic fixture only.'
 result=invoke('narrative_scene_save',{'scene':scene,'expected':{'kind':'stamp','stamp':file['stamp']},'slug':file['slug']},True)
 result['label']='guarded_scene_save';records.append(result)
 (OUT/'action-samples.json').write_text(json.dumps(records,indent=2)+'\n')
 (OUT/'action-memory.json').write_text(json.dumps(memory(pid),indent=2)+'\n')
 print('save',round(result['ms'],1),max(result['frameIntervals'],default=0),flush=True)

if __name__=='__main__':
 phase=sys.argv[1];pid=int(sys.argv[2])
 metadata={'phase':phase,'time':time.time(),'loadavg':os.getloadavg(),'scope':'actual Tauri/WebKit public IPC; harness-assisted calls, no provider calls and no mocked IPC','processes':subprocess.check_output(['ps','-eo','pid,comm,pcpu,pmem','--sort=-pcpu'],text=True).splitlines()[:20]}
 (OUT/f'{phase}-environment.json').write_text(json.dumps(metadata,indent=2)+'\n')
 {'reads':read_phase,'actions':action_phase}[phase](pid)

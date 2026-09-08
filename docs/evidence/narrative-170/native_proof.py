"""Native #170 proof. Run only after an explicit harness handoff; no provider action."""
import hashlib, json, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, '/tmp/wobu-acceptance-20260908')
from driver import send, root, shot

def invoke(command, **args): return send('invoke', command=command, args=args)
def record(name, value):
    (root / (name + '.json')).write_text(json.dumps(value, indent=2))
    return value

def wait_text(text):
    for _ in range(120):
        current = send('inspect')
        if text in current['text']: return current
        time.sleep(.25)
    raise AssertionError('Missing UI text: ' + text)

def capture(name):
    state = record(name, send('inspect'))
    assert not state['errors'], state['errors']
    shot(name)
    return state

def hashes(project):
    return {str(p.relative_to(project)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in (project / 'narrative' / 'scenes').glob('*.yaml')}

def run(project, window):
    invoke('project_open', path=str(project)); send('reload'); time.sleep(1.5)
    for _ in range(120):
        if send('inspect', selector='button[aria-label="Narrative"]')['elements']: break
        time.sleep(.25)
    send('hideTestOnboarding'); send('click', selector='button[aria-label="Narrative"]')
    wait_text('Variants…')
    scenes = invoke('narrative_scenes')['scenes']
    assert len(scenes) == 1, scenes
    scene = invoke('narrative_scene_get', sceneId=scenes[0]['id'])['scene']
    beat = scene['beats'][0]
    send('click', selector=f'button[data-library-scene="{scene["id"]}"][data-library-tab="script"]')
    for _ in range(120):
        button = next((e for e in send('inspect')['elements'] if e['text'] == 'Variants…'), None)
        if button and not button['disabled']: break
        time.sleep(.25)
    send('click', text='Variants…'); wait_text('Save policy and plan')
    for _ in range(120):
        button = next((e for e in send('inspect')['elements'] if e['text'] == 'Save policy and plan'), None)
        if button and not button['disabled']: break
        time.sleep(.25)
    before = hashes(project)
    send('click', text='Save policy and plan')
    wait_text('Potential 12 · Included 7 · Excluded 5 · Unknown 0')
    view = record('variants170-planned', invoke('narrative_variants_get', scene=scene['id'], beat=beat['id']))
    saved = view['latest']; assert saved and not view['stale']
    report = saved['report']
    assert [report[k]['value'] for k in ['potential','included','excluded','unknown']] == ['12','7','5','0']
    included = [r for r in report['rows'] if r['classification'] == 'included']
    assert len(included) == 7 and all(not r['witness']['runtime_route_verified'] for r in included)
    assert hashes(project) == before, 'Keyless planning changed scene source'
    send('click', selector='.variants-table details summary'); capture('variants170-matrix')
    send('click', selector='.narrative-variants header button', text='Close')
    send('click', text='Variants…'); wait_text('Potential 12 · Included 7 · Excluded 5 · Unknown 0')
    reopened = record('variants170-reopened-state', invoke('narrative_variants_get', scene=scene['id'], beat=beat['id']))
    assert reopened['latest']['id'] == saved['id']; capture('variants170-reopened')
    subprocess.run(['xdotool','windowsize',window,'960','620'],env={'DISPLAY':':97'},check=True)
    time.sleep(.4)
    geometry = record('variants170-960-geometry',send('inspect',selector='.narrative-variants-modal'))['elements'][0]['rect']
    assert geometry['x'] >= 0 and geometry['y'] >= 0 and geometry['x']+geometry['width'] <= 960 and geometry['y']+geometry['height'] <= 620, geometry
    capture('variants170-960')
    send('scroll',selector='.narrative-variants',top=99999)
    time.sleep(.2)
    capture('variants170-960-actions')
    subprocess.run(['xdotool','windowsize',window,'1440','900'],env={'DISPLAY':':97'},check=True)
    time.sleep(.4)
    send('scroll',selector='.narrative-variants',top=0)
    send('input',selector='select[aria-label="Target line"]',value=beat['dialogue'][1]['id'])
    send('click',text='Select all included (7)'); wait_text('Generation count: 7')
    send('click',text='Materialize 7 variants and open Build'); wait_text('7 items ·')
    builds=invoke('narrative_build_list');status=record('variants170-build-status',invoke('narrative_build_status',buildId=builds[0]['id']))
    assert len(status['build']['items']) == 7 and all(i['action']=='generate' and i['target']['variant'] for i in status['build']['items'])
    assert len({json.dumps(i['state'],sort_keys=True) for i in status['build']['items']}) == 7
    assert not status['dispatched'] and all(h['attempts']==0 for h in status['history'])
    capture('variants170-build')
    updated=record('variants170-materialized',invoke('narrative_scene_get',sceneId=scene['id']))['scene']
    assert len(updated['beats'][0]['dialogue'][1]['variants']) == 7
    def wording(v): return (v['id'],v.get('when'),v['text']['body'],v['text']['revision'],v['text']['provenance'],v['text'].get('lifecycle',{}).get('policy'))
    assert list(map(wording,updated['beats'][0]['dialogue'][0]['variants'])) == list(map(wording,scene['beats'][0]['dialogue'][0]['variants']))
    print(json.dumps({'report':saved['id'],'build':status['build']['id'],'potential':12,'included':7,'excluded':5,'unknown':0,'provider_calls':0,'window_restored':'1440x900'},indent=2))

if __name__ == '__main__': run(Path(sys.argv[1]), sys.argv[2] if len(sys.argv)>2 else '0x200003')

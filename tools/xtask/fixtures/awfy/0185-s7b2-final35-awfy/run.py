from pathlib import Path
import subprocess,json,hashlib,os,shutil
P=Path(__file__).resolve().parent;Q=P.parent;S=Q/'0185-s7b2-dev35/repo';sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest();pins=json.loads((P/'SOURCE-PINS.json').read_text());assert all(sha(S/f)==h for f,h in pins.items());T=P/'target';assert not T.exists();env=dict(os.environ,CARGO_TARGET_DIR=str(T),CARGO_INCREMENTAL='0');argv=['/home/me/.cargo/bin/cargo','+1.98.1','build','--locked','--features','native','--message-format=json','-j8']
with(P/'build.stdout').open('xb')as o,(P/'build.stderr').open('xb')as e:r=subprocess.run(argv,cwd=P,env=env,stdout=o,stderr=e,timeout=2400)
assert r.returncode==0,r.returncode;rows=[json.loads(l)for l in(P/'build.stdout').read_text().splitlines()if l.startswith('{')];units=[r for r in rows if r.get('reason')=='compiler-artifact'and r.get('target',{}).get('name')=='rune'];assert units and all(u['manifest_path']==str(S/'crates/rune/Cargo.toml')and not u['fresh']for u in units);B=P/'bin-after';shutil.copy2(T/'debug/awfy-runner',B);(P/'BINARY.json').write_text(json.dumps(dict(sha256=sha(B),source=str(S),rune_units=units),indent=2)+'\n')
allowance=json.loads((P/'HAVLAK-DIAGNOSTIC-ALLOWANCE.json').read_text());assert allowance['approval']=='01a16d28-9e5f-70ca-9448-9c90e9cce68b'
old=json.loads((Q/'0185-c18b3f-awfy1/OBSERVATIONS.json').read_text());assert len(old)==24
input_pins=json.loads((Q/'0185-c17f-awfy1/INPUT-PINS.json').read_text());ports=Q/'0185-c17f-awfy1/ports';port_pins={f.name:sha(f)for f in ports.glob('*.rn')};assert all(sha(S/'benches/awfy'/n)==h==input_pins[str(ports/n)] for n,h in port_pins.items())
oracle_files=[Q/'0185-c18b3f-awfy1/OBSERVATIONS.json',Q/'0185-c17f-awfy1/FRONTIER.json',Q/'0185-c17f-awfy1/INPUT-PINS.json',Q/'0185-c18b3f-l1-2/L1-CLASSIFICATION.json'];oracle_pins={str(f):sha(f)for f in oracle_files};(P/'ORACLE-PINS.json').write_text(json.dumps(oracle_pins,indent=2)+'\n')
legacy=json.loads((Q/'0185-c17f-awfy1/FRONTIER.json').read_text());out=[]
for prior in old:
 name=prior['program'];outer=prior['outer'];reference=legacy['programs'][name][str(outer)]['stock'];observations={'stock':prior['observations']['stock'],'published_c18b3f':prior['observations']['candidate']}
 for label,binary in [('candidate',B)]:
  argv=[str(binary),*reference['argv'][1:3],*[str(ports/Path(f).name) for f in reference['argv'][3:]]]
  r=subprocess.run(argv,capture_output=True,timeout=600);prefix=P/f'{name}-{outer}-{label}';prefix.with_suffix('.stdout').write_bytes(r.stdout);prefix.with_suffix('.stderr').write_bytes(r.stderr);observations[label]=dict(rc=r.returncode,stdout=r.stdout.decode(),stderr=r.stderr.decode());print('AWFY_RUN',name,outer,label,r.returncode,flush=True)
 classification=prior['classification']
 if name=='havlak':
  assert outer in [1,2] and reference['argv'][1:3]==[str(outer),'1']
  assert all(sha(ports/n)==h for n,h in allowance['sources'].items())
  assert classification=='Known-gap',(name,classification)
  assert observations['candidate']['rc']==0 and observations['candidate']['stdout']==observations['stock']['stdout'] and observations['candidate']['stderr']==f'fresh live_entries={2099908 * outer}\n',(name,outer,observations)
  classification='Exact'
 else:
  assert observations['candidate']==observations['published_c18b3f'],(name,outer,observations)
  if classification=='Exact':assert observations['candidate']['rc']==0 and observations['candidate']['stdout']==observations['stock']['stdout'] and observations['candidate']['stderr']=='fresh live_entries=0\n'
 out.append(dict(program=name,outer=outer,classification=classification,observations=observations));(P/'OBSERVATIONS.json').write_text(json.dumps(out,indent=2)+'\n')
assert sum(r['classification']=='Exact'for r in out)==20 and all(sha(S/f)==h for f,h in pins.items())
(P/'RESULT.json').write_text(json.dumps(dict(status='PASS_HAVLAK_STOCK_EXACT',source_export=str(S),source_pins_sha256=sha(P/'SOURCE-PINS.json'),oracle_pins=oracle_pins,exact_programs=10,programs=12,runs=24,baseline_reexecutions=0,unchanged_except='Havlak default-budget progression',default_limits=True,ports=port_pins,observations=out),indent=2)+'\n')
print('S7A_AWFY10_OF12_EXACT',flush=True)
# The two separately approved L1 progressions must retain their new fatal diagnostic.
classification=json.loads((Q/'0185-c18b3f-l1-2/L1-CLASSIFICATION.json').read_text())
assert classification['status']=='PASS'
inputs=json.loads((Q/'0185-c2-l1/cases.json').read_text());checks=[]
for case in ['00063','00357']:
 source=Path(inputs[int(case)]['path']);text=source.read_text();assert sha(source)=='107f3a3577878a86e0204d64b987470e3033810cbdd40fd9f75866415619de0f'
 assert text[1057:1072]=='ll.push_back(1)'and text[1189:1198]=='it.next()'
 row=next(r for r in classification['named_refusal_progressions']if r['case']==case);assert row['before']==[1057,1072]and row['after']==[1189,1198]
 z=subprocess.run([str(B),'1','1',str(source)],capture_output=True,timeout=600);prefix=P/('l1-'+case);prefix.with_suffix('.stdout').write_bytes(z.stdout);prefix.with_suffix('.stderr').write_bytes(z.stderr)
 assert z.returncode==3 and 'kind=Method'in z.stdout.decode()and '"it.next()"'in z.stdout.decode()
 assert 'error: Method'in z.stderr.decode()and 'it.next()'in z.stderr.decode(),(case,z.stderr.decode())
 checks.append(dict(case=case,source_sha256=sha(source),old_span=[1057,1072],new_span=[1189,1198],real_span_progression=True,new_fatal_diagnostic=True,stdout_sha256=sha(prefix.with_suffix('.stdout')),stderr_sha256=sha(prefix.with_suffix('.stderr'))))
(P/'L1-DIAGNOSTIC-CHECK.json').write_text(json.dumps(dict(status='PASS',checks=checks),indent=2)+'\n')
print('L1_NEW_REFUSALS_CARRY_FATAL_DIAGNOSTICS2',flush=True)

controls=[]
for prior in [r for r in old if r['program']=='havlak']:
 outer=prior['outer'];reference=legacy['programs']['havlak'][str(outer)]['stock'];argv=[str(B),*reference['argv'][1:3],*[str(ports/Path(f).name)for f in reference['argv'][3:]]]
 for kind,extra in [('frames',dict(S7A_FRAME_LIMIT='1024')),('slots',dict(S7A_SLOT_LIMIT='65536')),('both',dict(S7A_FRAME_LIMIT='1024',S7A_SLOT_LIMIT='65536'))]:
  z=subprocess.run(argv,capture_output=True,env=dict(os.environ,**extra),timeout=600);name=f'havlak-{outer}-control-{kind}';(P/(name+'.stdout')).write_bytes(z.stdout);(P/(name+'.stderr')).write_bytes(z.stderr);controls.append(dict(outer=outer,kind=kind,env=extra,rc=z.returncode,stdout_sha256=sha(P/(name+'.stdout')),stderr_sha256=sha(P/(name+'.stderr')),stderr=z.stderr.decode()));print('S7A_HAVLAK_CONTROL',outer,kind,z.returncode,flush=True)
(P/'HAVLAK-LIMIT-CONTROLS.json').write_text(json.dumps(controls,indent=2)+'\n')
assert all(sha(S/f)==h for f,h in pins.items())

assert all(sha(f)==h for f,h in oracle_pins.items())

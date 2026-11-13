#!/usr/bin/env python3
import json
from pathlib import Path
def ts(s):
 if '$ref'in s:return s['$ref'].split('/')[-1]
 if 'enum'in s:return ' | '.join(json.dumps(x) for x in s['enum'])
 t=s.get('type')
 if isinstance(t,list):return '('+' | '.join(ts({**s,'type':kind}) for kind in t)+')'
 if t=='object' and not s.get('properties') and isinstance(s.get('additionalProperties'),dict):return 'Record<string, '+ts(s['additionalProperties'])+'>'
 if t=='object':return '{ '+ '; '.join(json.dumps(k)+('' if k in s.get('required',[]) else '?')+': '+ts(v) for k,v in s.get('properties',{}).items())+' }'
 if t=='array':return '('+ts(s['items'])+')[]'
 return {'string':'string','integer':'number','number':'number','boolean':'boolean','null':'null'}.get(t,'unknown')
def main():
 spec=json.loads(Path('docs/openapi.json').read_text())
 Path('web/src/api.generated.ts').write_text('// Generated from docs/openapi.json. Run python3 scripts/generate-types.py.\n'+''.join('export type '+k+' = '+ts(v)+';\n' for k,v in spec['components']['schemas'].items()))
if __name__=='__main__':main()

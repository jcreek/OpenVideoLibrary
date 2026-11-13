#!/usr/bin/env python3
"""Save compact browser acceptance evidence without ephemeral paths or traces."""
import argparse
import datetime
import json
from pathlib import Path
import platform


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input',default='test-results/browser-results.json')
    parser.add_argument('--output',required=True)
    args=parser.parse_args()
    report=json.loads(Path(args.input).read_text())
    results=[]
    def walk(suite):
        for spec in suite.get('specs',[]):
            for test in spec.get('tests',[]):
                versions=[a['description'] for a in test.get('annotations',[]) if a['type']=='browser-version']
                results.append({'project':test['projectName'],'browser_versions':versions,'test':spec['title'],
                                'status':test['status'],'runs':[{'status':r['status'],'duration_ms':r['duration']} for r in test['results']]})
        for child in suite.get('suites',[]):walk(child)
    for suite in report['suites']:walk(suite)
    output={'recorded_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'host':platform.platform(),
            'note':'Desktop engine automation; not real mobile/TV certification or GPU acceptance.',
            'results':results,'stats':report['stats']}
    Path(args.output).write_text(json.dumps(output,indent=2)+'\n')
    if not results or any(r['status']!='expected' for r in results):raise SystemExit(1)


if __name__=='__main__':main()

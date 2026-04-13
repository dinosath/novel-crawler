#!/usr/bin/env python3
import json
import re
import os
from collections import defaultdict

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..'))
NOVEL_JSON = os.path.join(ROOT, 'Lightnovels', 'The_Best_Tamer_of_All_Time_With_Max_Affinity', 'The_Best_Tamer_of_All_Time_With_Max_Affinity.json')
GAME_DB = os.path.join(ROOT, 'Lightnovels', 'The_Best_Tamer_of_All_Time_With_Max_Affinity', 'game_database.json')
AUDIT_OUT = os.path.join(ROOT, 'Lightnovels', 'The_Best_Tamer_of_All_Time_With_Max_Affinity', 'mention_audit.json')


def collect_entity_names(obj):
    names = set()
    def walk(x):
        if isinstance(x, dict):
            for k,v in x.items():
                if k in ('name','title') and isinstance(v, str):
                    names.add(v)
                if k in ('aliases','aliases_list') and isinstance(v, (list,tuple)):
                    for a in v:
                        if isinstance(a,str): names.add(a)
            for v in x.values():
                walk(v)
        elif isinstance(x, list):
            for i in x: walk(i)
    walk(obj)
    return names


def strip_html(s):
    return re.sub(r'<[^>]+>', '', s)


def find_mentions_in_text(text, names):
    hits = defaultdict(list)
    lowered = text
    for name in sorted(names, key=len, reverse=True):
        if not name.strip():
            continue
        # use case-insensitive search with boundary heuristics
        esc = re.escape(name)
        pattern = re.compile(r'(?i)\b' + esc + r'\b')
        for m in pattern.finditer(lowered):
            start, end = m.start(), m.end()
            snippet = lowered[max(0, start-40):min(len(lowered), end+40)].strip()
            hits[name].append({'chapter_pos': start, 'chapter_span': [start,end], 'snippet': snippet})
    return hits


def main():
    print('Loading novel and game DB...')
    novel = json.load(open(NOVEL_JSON, 'r', encoding='utf-8'))
    game = json.load(open(GAME_DB, 'r', encoding='utf-8'))

    print('Collecting entity names from game database...')
    entity_names = collect_entity_names(game)
    print(f'Found {len(entity_names)} named entries in game DB.')

    # detect whether entities have mentions[] fields
    def collect_entities_with_mentions(obj, path=''):
        out = {}
        if isinstance(obj, dict):
            if 'name' in obj and isinstance(obj['name'], str):
                n = obj['name']
                out[n] = {'path': path or '/', 'has_mentions': ('mentions' in obj and isinstance(obj['mentions'], list)), 'mentions_count': len(obj.get('mentions', [])) if isinstance(obj.get('mentions', []), list) else 0}
            for k,v in obj.items():
                newpath = f"{path}/{k}" if path else k
                out.update(collect_entities_with_mentions(v, newpath))
        elif isinstance(obj, list):
            for i,el in enumerate(obj):
                newpath = f"{path}[{i}]"
                out.update(collect_entities_with_mentions(el, newpath))
        return out

    entities_info = collect_entities_with_mentions(game)

    # scan chapters
    chapters = novel.get('chapters', [])
    print(f'Scanning {len(chapters)} chapters for mentions (this may take a moment)...')

    orphan_names = set()
    mentions_found = defaultdict(list)

    # prebuild name variants lower-cased for better matching
    names_for_scan = set(entity_names)

    for ch in chapters:
        cid = ch.get('id')
        body_html = ch.get('body','')
        body = strip_html(body_html)
        body = body.replace('\u00a0', ' ')
        found = find_mentions_in_text(body, names_for_scan)
        for n,hits in found.items():
            for h in hits:
                h['chapter_id'] = cid
                mentions_found[n].append(h)

    # find orphan mentions (names found in text but not in game DB)
    # also detect entities in DB missing mentions arrays
    orphan_in_text = [n for n in mentions_found.keys() if n not in entity_names]
    missing_mentions_records = []
    for n,info in entities_info.items():
        has = info.get('has_mentions', False)
        count = info.get('mentions_count', 0)
        found_count = len(mentions_found.get(n, []))
        if not has and found_count>0:
            missing_mentions_records.append({'name': n, 'path': info.get('path'), 'mentions_found_in_text': found_count})

    audit = {
        'scanned_chapters': len(chapters),
        'entities_in_game_db_count': len(entity_names),
        'entities_with_missing_mentions_field_but_found_in_text': missing_mentions_records,
        'orphan_names_found_in_text_not_in_db': [{'name': n, 'count': len(mentions_found[n]), 'sample': mentions_found[n][:3]} for n in orphan_in_text],
        'entities_summary': {},
    }

    for n in sorted(entity_names):
        info = entities_info.get(n, {})
        audit['entities_summary'][n] = {
            'path_in_db': info.get('path'),
            'has_mentions_field': info.get('has_mentions', False),
            'mentions_recorded_count': info.get('mentions_count', 0),
            'mentions_found_in_text_count': len(mentions_found.get(n, [])),
            'sample_text_mentions': mentions_found.get(n, [])[:3]
        }

    # write audit
    with open(AUDIT_OUT, 'w', encoding='utf-8') as f:
        json.dump(audit, f, ensure_ascii=False, indent=2)

    print('Audit written to', AUDIT_OUT)
    print('Summary:')
    print('-', 'Entities in DB:', len(entity_names))
    print('-', 'Entities missing mentions[] but found in text:', len(missing_mentions_records))
    print('-', 'Orphan names found in text not in DB:', len(orphan_in_text))

if __name__ == '__main__':
    main()

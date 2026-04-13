#!/usr/bin/env python3
import json
import os
import re
from hashlib import sha1
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NOVEL_JSON = ROOT / 'Lightnovels' / 'The_Best_Tamer_of_All_Time_With_Max_Affinity' / 'The_Best_Tamer_of_All_Time_With_Max_Affinity.json'
MENTION_AUDIT = ROOT / 'Lightnovels' / 'The_Best_Tamer_of_All_Time_With_Max_Affinity' / 'mention_audit.json'
OUT_DB = ROOT / 'Lightnovels' / 'The_Best_Tamer_of_All_Time_With_Max_Affinity' / 'game_database.json'


def slug(s):
    s = re.sub(r"[^0-9a-zA-Z -]", '', s)
    s = re.sub(r"\s+", '-', s.strip())
    return s.lower()


def make_id(prefix, name):
    h = sha1(name.encode('utf-8')).hexdigest()[:10]
    return f"{prefix}_{h}"


def infer_type_from_path(path):
    path = path or ''
    if path.startswith('characters'):
        return 'character'
    if path.startswith('items'):
        return 'item'
    if path.startswith('locations'):
        return 'location'
    if path.startswith('factions'):
        return 'faction'
    if path.startswith('skills') or 'skills' in path:
        return 'skill'
    return 'entity'


def build_db():
    novel = json.load(open(NOVEL_JSON, 'r', encoding='utf-8'))
    audit = json.load(open(MENTION_AUDIT, 'r', encoding='utf-8'))

    chapters = novel.get('chapters', [])
    metadata = {
        'title': novel.get('title'),
        'url': novel.get('url'),
        'author': novel.get('author'),
        'chapters_analyzed': audit.get('scanned_chapters', len(chapters)),
        'source': audit.get('source', 'novelishuniverse.com'),
        'generated_by': 'scripts/generate_game_database.py'
    }

    entities = []
    mentions_index = {}
    mention_seq = 1

    entities_summary = audit.get('entities_summary', {})

    for name,summary in entities_summary.items():
        ent_type = infer_type_from_path((summary.get('path_in_db') or '').lstrip('/'))
        eid = make_id(ent_type, name)
        mentions = []
        for m in summary.get('sample_text_mentions', []):
            # the audit stored snippets and chapter spans; for determinism include them
            mid = f"m{mention_seq:08d}"
            mention_seq += 1
            mention_obj = {
                'mention_id': mid,
                'chapter_id': m.get('chapter_id'),
                'position': m.get('chapter_pos'),
                'span': m.get('chapter_span'),
                'snippet': m.get('snippet')
            }
            mentions.append(mention_obj)
            mentions_index[mid] = {
                'entity_id': eid,
                'entity_name': name,
                'chapter_id': m.get('chapter_id'),
                'position': m.get('chapter_pos'),
                'span': m.get('chapter_span'),
                'snippet': m.get('snippet')
            }
        ent = {
            'id': eid,
            'name': name,
            'type': ent_type,
            'path_in_previous_db': summary.get('path_in_db'),
            'mentions': mentions,
            'mentions_count_recorded': summary.get('mentions_recorded_count', 0),
            'mentions_found_in_text_count': summary.get('mentions_found_in_text_count', 0)
        }
        entities.append(ent)

    # For entities found in audit['entities_with_missing_mentions_field_but_found_in_text'], we should expand mentions from full audit file
    # Load full mention details by re-scanning novel to pick all occurrences for those names
    # Build quick name->entity_id map
    name_to_eid = {e['name']: e['id'] for e in entities}

    # prepare name list to expand
    expand_names = [x['name'] for x in audit.get('entities_with_missing_mentions_field_but_found_in_text', [])]

    if expand_names:
        # load chapters text
        for ch in chapters:
            cid = ch.get('id')
            body = re.sub(r'<[^>]+>', '', ch.get('body',''))
            for name in expand_names:
                # case-insensitive, whole-word
                pattern = re.compile(r'(?i)\b' + re.escape(name) + r'\b')
                for m in pattern.finditer(body):
                    pos = m.start()
                    span = [m.start(), m.end()]
                    snippet = body[max(0,pos-60):min(len(body), pos+60)].strip()
                    mid = f"m{mention_seq:08d}"
                    mention_seq += 1
                    mentions_index[mid] = {
                        'entity_id': name_to_eid[name],
                        'entity_name': name,
                        'chapter_id': cid,
                        'position': pos,
                        'span': span,
                        'snippet': snippet
                    }
                    # append to entity mentions
                    for ent in entities:
                        if ent['name'] == name:
                            ent['mentions'].append({
                                'mention_id': mid,
                                'chapter_id': cid,
                                'position': pos,
                                'span': span,
                                'snippet': snippet
                            })
                            break

    # now produce global mentions_index list (dict)
    db = {
        'metadata': metadata,
        'entities': entities,
        'mentions_index': mentions_index,
        'summary': {
            'entities_count': len(entities),
            'total_mentions_indexed': len(mentions_index)
        }
    }

    # write
    with open(OUT_DB, 'w', encoding='utf-8') as f:
        json.dump(db, f, ensure_ascii=False, indent=2)

    print('Wrote new game database to', OUT_DB)
    print('Entities:', len(entities), 'Mentions indexed:', len(mentions_index))


if __name__ == '__main__':
    build_db()

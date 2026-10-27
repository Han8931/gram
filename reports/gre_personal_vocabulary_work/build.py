"""Assemble the reviewed personal vocabulary cards without changing study data."""

import csv
import json
import re
import unicodedata
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = Path(__file__).resolve().parent
FIELDS = ['term', 'pron', 'pos', 'definition', 'example', 'notes']


def normalize(text):
    return ' '.join(unicodedata.normalize('NFKC', text).casefold().split())


def read_csv(path):
    with path.open(encoding='utf-8-sig', newline='') as stream:
        return list(csv.DictReader(stream))


def normalize_pos(text):
    basic = {
        'noun': 'n', 'verb': 'v', 'adjective': 'adj', 'adverb': 'adv',
        'preposition': 'prep', 'conjunction': 'conj', 'pronoun': 'pron',
    }
    expanded = {'n': 'noun', 'v': 'verb', 'adj': 'adjective', 'adv': 'adverbial', 'prep': 'prepositional', 'conj': 'conjunction'}
    result = []
    for part in re.split('[;,]', text):
        part = ' '.join(part.replace('.', '').split())
        part = re.sub(r'\bphr\b', 'phrase', part)
        if part in {'n pl', 'plural n'}:
            part = 'plural n'
        elif ' ' in part:
            first, rest = part.split(' ', 1)
            part = expanded.get(first, first) + ' ' + rest
        else:
            part = basic.get(part, part)
        if part not in result:
            result.append(part)
    return ', '.join(result)


def main():
    source = read_csv(ROOT / 'reports/gre_xlsx_missing_entries.csv')
    existing = defaultdict(list)
    for filename in ['word_smart.csv', 'GRE_Vocab.csv']:
        for card in read_csv(ROOT / 'decks' / filename):
            existing[normalize(card['term'])].append(filename)
    drafts = {}
    for shard in 'abcde':
        path = WORK / f'draft_{shard}.jsonl'
        for line in path.read_text().splitlines():
            card = json.loads(line)
            index = card['id']
            assert isinstance(index, int) and 0 <= index < len(source), (path, index)
            assert index not in drafts, ('duplicate source ID', index)
            for field in ['term', 'pos', 'definition', 'example']:
                assert isinstance(card[field], str) and card[field].strip(), (index, field)
                card[field] = card[field].strip()
            assert not re.search('[가-힣]', card['definition'] + card['example']), index
            highlights = re.findall(r'\*([^*]+)\*', card['example'])
            assert highlights and card['example'].count('*') == 2 * len(highlights), index
            assert '\n' not in card['term'], index
            assert not re.search('[가-힣]', card['term']), index
            card['pos'] = normalize_pos(card['pos'])
            drafts[index] = card
    assert set(drafts) == set(range(len(source))), ('missing IDs', sorted(set(range(len(source))) - set(drafts)))

    cards = {}
    audit = []
    for index in range(len(source)):
        original = source[index]
        draft = drafts[index]
        key = normalize(draft['term'])
        notes = [
            'Personal GRE study notes.',
            'Source: GRE단어장.xlsx; ' + original['locations'],
            'Original Korean notes: ' + (original['definition'].strip() or '(none)'),
        ]
        if draft['term'] != original['term']:
            notes.append('Original entry: ' + original['term'])
        if draft.get('usage_note', '').strip():
            notes.append('Usage note: ' + draft['usage_note'].strip())
        note = '\n'.join(notes)
        # Retain corrected source entries even when the correction reveals an
        # existing headword: the user's notes may concern a different sense.
        # Document that overlap instead of silently losing the personal note.
        if existing.get(key):
            note += '\nAlso a headword in: ' + ', '.join(sorted(set(existing[key])))
        disposition = 'new card'
        if key in cards:
            disposition = 'merged with normalized duplicate'
            card = cards[key]
            for field in ['definition', 'example']:
                if draft[field] not in card[field].split('\n'):
                    card[field] += '\n' + draft[field]
            positions = list(dict.fromkeys(card['pos'].split(', ') + draft['pos'].split(', ')))
            card['pos'] = ', '.join(positions)
            card['notes'] += '\n\n' + note
        else:
            cards[key] = {field: draft[field] for field in ['term', 'pos', 'definition', 'example']}
            cards[key].update(pron='', notes=note)
        audit.append({
            'source_id': index,
            'original_term': original['term'],
            'deck_term': cards[key]['term'],
            'disposition': disposition,
            'existing_headword_overlap': '; '.join(sorted(set(existing.get(key, [])))),
            'locations': original['locations'],
            'usage_note': draft.get('usage_note', ''),
        })

    output = ROOT / 'decks/GRE_Personal_Vocabulary.csv'
    with output.open('w', encoding='utf-8', newline='') as stream:
        writer = csv.DictWriter(stream, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(cards[key] for key in sorted(cards))
    audit_path = ROOT / 'reports/gre_personal_vocabulary_audit.csv'
    with audit_path.open('w', encoding='utf-8-sig', newline='') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(audit[0]))
        writer.writeheader()
        writer.writerows(audit)
    summary = {
        'source_entries': len(source),
        'deck_cards': len(cards),
        'merged_entries': len(source) - len(cards),
        'renamed_source_entries': sum(row['original_term'] != row['deck_term'] for row in audit),
        'corrected_headwords_also_in_existing_decks': sum(bool(existing.get(key)) for key in cards),
    }
    (WORK / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()

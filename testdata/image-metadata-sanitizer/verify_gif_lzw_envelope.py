#!/usr/bin/env python3
"""CC0 code-word controls, not an encoder or decoder shared with the rewriter.

Literal ordinal j allocates entry clear+1+j (j>=1). Thus widths and
entry contents can be written in closed form, without a dictionary state
machine. Golden pixels are authored literals and explicitly known two-symbol
strings; Pillow and ImageMagick independently check the variable-bit packing.
"""
import hashlib
import json
import struct
import subprocess
import sys
from pathlib import Path
from PIL import Image


def pack(words):
    # Explicit word widths, LSB first; no width inference from code values.
    bits = ''.join(''.join(str((code >> bit) & 1) for bit in range(width))
                   for code, width in words)
    bits += '0' * (-len(bits) % 8)
    return bytes(sum(int(bits[start + bit]) << bit for bit in range(8))
                 for start in range(0, len(bits), 8))


def blocks(data):
    return b''.join(bytes([len(data[i:i + 255])]) + data[i:i + 255]
                    for i in range(0, len(data), 255)) + b'\0'


def literal_prefix(minimum, count):
    clear = 1 << minimum
    words = [(clear, minimum + 1)]
    pixels = [j % clear for j in range(count)]
    words += [(pixel, min(12, max(minimum + 1, (clear + 2 + max(0, j - 1)).bit_length())))
              for j, pixel in enumerate(pixels)]
    return words, pixels


def artwork(minimum, words, pixels):
    colors = 1 << minimum
    palette = bytes(component for i in range(colors)
                    for component in (i, (i * 53 + 17) % 256, (i * 97 + 31) % 256))
    header = b'GIF87a' + struct.pack('<HHBBB', len(pixels), 1, 128 | (minimum - 1), 0, 0)
    data = header + palette + b',' + struct.pack('<HHHHB', 0, 0, len(pixels), 1, 0)
    data += bytes([minimum]) + blocks(pack(words)) + b';'
    rgba = bytes(component for index in pixels for component in (*palette[index * 3:index * 3 + 3], 255))
    return data, palette, rgba


def controls():
    for minimum in range(2, 9):
        clear = 1 << minimum
        # Migrate the formerly valid-but-unsupported minimum controls verbatim.
        if minimum > 2:
            words = [(clear, minimum + 1), (0, minimum + 1), (1, minimum + 1),
                     (2, minimum + 1), (clear + 1, minimum + 1)]
            yield f'migrated-minimum-{minimum}', minimum, words, [0, 1, 2], {'highest_allocated': clear + 3}
        for width in range(minimum + 1, 13):
            # Allocation immediately below a width boundary (or full table).
            count = (1 << width) - clear - 2
            for side in ('before', 'after'):
                n = count + (side == 'after')
                words, pixels = literal_prefix(minimum, n)
                highest = clear + n
                for ending in ('end', 'reset', 'KwKwK', 'entry'):
                    tail_width = min(12, width + (side == 'after'))
                    schedule = list(words)
                    golden = list(pixels)
                    if ending == 'KwKwK':
                        if highest == 4095:
                            # At full table no next-code special case exists.
                            continue
                        schedule.append((highest + 1, tail_width))
                        golden += [pixels[-1]] * 2
                        tail_width = min(12, max(tail_width, (highest + 2).bit_length()))
                    if ending == 'entry':
                        schedule.append((highest, tail_width))
                        golden += [(n - 2) % clear, (n - 1) % clear]
                        tail_width = min(12, max(tail_width, (highest + 2).bit_length()))
                    if ending == 'reset':
                        schedule += [(clear, tail_width), (0, minimum + 1),
                                     (clear + 2, minimum + 1)]
                        golden += [0, 0, 0]  # reset followed by KwKwK
                        tail_width = minimum + 1
                    schedule.append((clear + 1, tail_width))
                    yield f'm{minimum}-w{width}-{side}-{ending}', minimum, schedule, golden, {
                        'boundary_width': width, 'side': side, 'ending': ending,
                        'highest_allocated': min(4095, highest + (ending in ('KwKwK', 'entry'))),
                        'literal_count_before_tail': n,
                        'transition': f'{width}->{width + 1}' if side == 'after' and width < 12 else None,
                        'reset_from_width': min(12, width + (side == 'after')) if ending == 'reset' else None}
        # Consecutive specials use an allocated previous string, not just a
        # literal: code clear+2 => 00, clear+3 => 000, clear+4 => 0000.
        words = [(clear, minimum + 1), (0, minimum + 1)]
        words += [(clear + 2 + j, max(minimum + 1, (clear + 2 + j).bit_length())) for j in range(3)]
        words.append((clear + 1, max(minimum + 1, (clear + 5).bit_length())))
        yield f'm{minimum}-chained-KwKwK', minimum, words, [0] * 10, {
            'highest_allocated': clear + 4, 'special_lengths': [2, 3, 4]}
        # Full table remains frozen: use both last and earlier allocated entries,
        # literals, then last entry again before clearing. Entry k is the pair
        # of literals at ordinals k-clear-2 and k-clear-1.
        n = 4095 - clear
        words, pixels = literal_prefix(minimum, n)
        for code in (4095, clear + 2, 0, 4095):
            words.append((code, 12))
            pixels += ([0] if code == 0 else [(code - clear - 2) % clear, (code - clear - 1) % clear])
        words += [(clear, 12), (1, minimum + 1), (clear + 2, minimum + 1), (clear + 1, minimum + 1)]
        pixels += [1, 1, 1]
        yield f'm{minimum}-saturation-deferred-clear', minimum, words, pixels, {
            'highest_allocated': 4095, 'frozen_dictionary_codes': [4095, clear + 2, 0, 4095],
            'reset_from_width': 12, 'post_reset_KwKwK': True}
    words, pixels = literal_prefix(2, 11)
    words.append((5, 5))
    yield 'migrated-first-width5', 2, words, pixels, {'highest_allocated': 15, 'transition': '4->5'}


def malformed_controls():
    for minimum in range(2, 9):
        clear = 1 << minimum
        for label, words in (
                ('initial-unallocated', [(clear, minimum + 1), (clear + 2, minimum + 1)]),
                ('afterclear-unallocated', [(clear, minimum + 1), (0, minimum + 1),
                                           (clear, minimum + 1), (clear + 2, minimum + 1)]),
                ('no-initial-clear', [(0, minimum + 1), (clear + 1, minimum + 1)]),
                ('premature-end', [(clear, minimum + 1), (clear + 1, minimum + 1)])):
            yield f'm{minimum}-{label}', artwork(minimum, words, [0])[0]
        for width in range(minimum + 1, 13):
            # After crossing each boundary (or filling at 12), truncate the
            # terminal variable-width word, with sub-blocks otherwise complete.
            n = (1 << width) - clear - 1
            words, pixels = literal_prefix(minimum, n)
            tail = min(12, width + 1)
            words.append((clear + 1, tail))
            valid = artwork(minimum, words, pixels)[0]
            payload = pack(words)[:-1]
            prefix = valid[:13 + 3 * clear + 10 + 1]
            yield f'm{minimum}-w{width}-truncated-end', prefix + blocks(payload) + b';'
            if width < 12:
                forward = list(words[:-1]) + [(clear + n + 2, tail), (clear + 1, tail)]
                yield f'm{minimum}-w{width}-forward', artwork(minimum, forward, pixels + [0, 0])[0]
            overflow = list(words[:-1]) + [(0, tail), (clear + 1, tail)]
            yield f'm{minimum}-w{width}-expansion-overflow', artwork(minimum, overflow, pixels)[0]
            # Clearing must invalidate entries allocated before that reset.
            reset = list(words[:-1]) + [(clear, tail), (clear + 2, minimum + 1)]
            yield f'm{minimum}-w{width}-stale-after-reset', artwork(minimum, reset, pixels + [0])[0]
    valid = artwork(2, [(4, 3), (0, 3), (5, 3)], [0])[0]
    at = 13 + 12 + 10
    for minimum in (0, 1, 9, 12, 255):
        yield f'unsupported-minimum-{minimum}', valid[:at] + bytes([minimum]) + valid[at + 1:]


if __name__ == '__main__':
    root, magick, *rewrite_args = sys.argv[1:]
    work = Path(root) / 'lzw-envelope'
    work.mkdir(parents=True, exist_ok=True)
    results = []
    for name, minimum, words, pixels, witness in controls():
        data, palette, golden = artwork(minimum, words, pixels)
        source = work / (name + '.gif')
        source.write_bytes(data)
        # Structural schedule is retained separately from independent consumers.
        (work / (name + '.words.json')).write_text(json.dumps(words))
        (work / (name + '.golden.rgba')).write_bytes(golden)
        paths = [source]
        if rewrite_args:
            rewrite, icc = rewrite_args
            target, repeat, second = [work / (name + suffix + '.gif') for suffix in ('.output', '.repeat', '.second')]
            for before, after in ((source, target), (source, repeat), (target, second)):
                subprocess.run([sys.executable, '-B', rewrite, str(before), str(after), icc], check=True)
                assert after.read_bytes() == data
            paths.append(target)
        for path in paths:
            with Image.open(path) as image:
                assert image.size == (len(pixels), 1) and image.n_frames == 1
                assert bytes(image.getpalette()[:len(palette)]) == palette
                assert image.tobytes() == bytes(pixels), (name, words[-4:], list(image.tobytes()[-12:]), pixels[-12:])
                assert image.convert('RGBA').tobytes() == golden
            raw = Path(str(path) + '.rgba')
            result = subprocess.run([magick, str(path), '-coalesce', '-depth', '8', 'rgba:' + str(raw)], capture_output=True, check=True)
            assert not result.stderr, result.stderr
            assert raw.read_bytes() == golden
        results.append({'control': name, 'minimum': minimum, 'widths': sorted({w for _, w in words}),
                        'pixels': len(pixels), 'structural_witness': witness,
                        'golden_sha256': hashlib.sha256(golden).hexdigest(),
                        'Pillow_exact_palette_indices_canvas': True, 'ImageMagick_exact_canvas': True,
                        'byte_preservation_determinism_idempotence': bool(rewrite_args)})
    assert len(results) == 406
    rejected = []
    if rewrite_args:
        for name, data in malformed_controls():
            source, target = work / (name + '.invalid.gif'), work / (name + '.invalid.output.gif')
            source.write_bytes(data)
            process = subprocess.run([sys.executable, '-B', rewrite, str(source), str(target), icc],
                                     capture_output=True, text=True)
            last = process.stderr.splitlines()[-1] if process.stderr else ''
            assert process.returncode == 1 and last.startswith('ValueError: invalid GIF '), (name, process.stderr)
            assert source.read_bytes() == data and not target.exists()
            rejected.append({'control': name, 'domain_error': last, 'unchanged_input_no_output': True})
        assert len(rejected) == 222
    print(json.dumps({'mode': 'rewrite' if rewrite_args else 'consumers-before-widening',
                      'count': len(results), 'controls': results, 'rejections': rejected}, indent=2))

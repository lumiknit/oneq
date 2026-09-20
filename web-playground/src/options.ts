// Split an option line into argv without invoking a shell or expanding values.
export const parseOptions = (source: string): string[] => {
  const args: string[] = [];
  let current = '';
  let quote: "'" | '"' | null = null;
  let escaped = false;
  let started = false;

  for (let i = 0; i < source.length; i++) {
    const char = source[i]!;

    if (escaped) {
      escaped = false;
      if (quote === '"' && !['"', '\\', '$', '`', '\n'].includes(char)) {
        current += '\\';
      }
      if (char !== '\n') current += char;
      started = true;
      continue;
    }

    if (char === '\\') {
      escaped = true;
      started = true;
      continue;
    }

    if (quote) {
      if (char === quote) {
        quote = null;
      } else {
        current += char;
      }
      continue;
    }

    if (char === "'" || char === '"') {
      quote = char;
      started = true;
      continue;
    }

    if (/\s/.test(char)) {
      if (started) {
        args.push(current);
        current = '';
        started = false;
      }
      continue;
    }

    current += char;
    started = true;
  }

  if (escaped) throw new Error('Options end with an unfinished escape.');
  if (quote) throw new Error('Options contain an unclosed quote.');
  if (started) args.push(current);

  return args;
};

export const quoteArg = (a: string): string => {
  if (a === '') return '""';
  if (/[\s"'\\]/.test(a)) return `"${a.replace(/[\\"]/g, (m) => `\\${m}`)}"`;
  return a;
};

export type CLIFormState = {
  from: string;
  to: string;
  input: 'stdin' | 'null' | 'doc';
  rawInput: boolean;
  slurp: boolean;
  stream: 'default' | 'stream' | 'stream-errors';
  outputMode: 'default' | 'raw' | 'raw0' | 'join';
  color: 'auto' | 'color' | 'mono';
  compact: 'pretty' | 'compact' | 'inline';
  tab: boolean;
  indent: string;
  ascii: boolean;
  sortKeys: boolean;
  exitStatus: boolean;
  quiet: boolean;
  unbuffered: boolean;
  seq: boolean;
  /** Anything not covered by a form field (e.g. --arg, -L, -f), kept verbatim. */
  extra: string;
};

export const defaultCLIFormState: CLIFormState = {
  from: 'json',
  to: 'json',
  input: 'stdin',
  rawInput: false,
  slurp: false,
  stream: 'default',
  outputMode: 'default',
  color: 'auto',
  compact: 'pretty',
  tab: false,
  indent: '',
  ascii: false,
  sortKeys: false,
  exitStatus: false,
  quiet: false,
  unbuffered: false,
  seq: false,
  extra: '',
};

export const FORMAT_OPTIONS: { value: string; label: string }[] = [
  { value: 'json', label: 'JSON' },
  { value: 'json5', label: 'JSON5' },
  { value: 'j', label: 'Loose JSON' },
  { value: 'pylit', label: 'Python literal' },
  { value: 'yaml', label: 'YAML' },
  { value: 'toml', label: 'TOML' },
  { value: 'xml', label: 'XML' },
  { value: 'csv', label: 'CSV' },
  { value: 'csvh', label: 'CSV (header row)' },
  { value: 'tsv', label: 'TSV' },
  { value: 'tsvh', label: 'TSV (header row)' },
  { value: 'raw', label: 'Raw (line by line)' },
  { value: 'rawslurp', label: 'Raw (whole input)' },
  { value: 'env', label: 'Env (KEY=VALUE)' },
  { value: 'exportenv', label: 'Env (export KEY=VALUE)' },
  { value: 'cbor', label: 'CBOR' },
  { value: 'jq', label: 'jq (AST)' },
];

export const serializeCLIFormState = (s: CLIFormState): string => {
  const args: string[] = [];
  if (s.from !== 'json') args.push('-F', s.from);
  if (s.to !== 'json') args.push('-T', s.to);
  if (s.input === 'null') args.push('-n');
  else if (s.input === 'doc') args.push('--doc');
  if (s.rawInput) args.push('-R');
  if (s.slurp) args.push('-s');
  if (s.stream === 'stream') args.push('--stream');
  else if (s.stream === 'stream-errors') args.push('--stream-errors');
  if (s.outputMode === 'raw') args.push('-r');
  else if (s.outputMode === 'raw0') args.push('--raw-output0');
  else if (s.outputMode === 'join') args.push('-j');
  if (s.color === 'color') args.push('-C');
  else if (s.color === 'mono') args.push('-M');
  if (s.compact === 'compact') args.push('-c');
  else if (s.compact === 'inline') args.push('--inline-output');
  if (s.tab) args.push('--tab');
  if (s.indent) args.push('--indent', s.indent);
  if (s.ascii) args.push('-a');
  if (s.sortKeys) args.push('-S');
  if (s.exitStatus) args.push('-e');
  if (s.quiet) args.push('-q');
  if (s.unbuffered) args.push('--unbuffered');
  if (s.seq) args.push('--seq');

  let extraTokens: string[];
  try {
    extraTokens = parseOptions(s.extra);
  } catch {
    extraTokens = s.extra.trim() ? [s.extra] : [];
  }
  args.push(...extraTokens);

  return args.map(quoteArg).join(' ');
};

const BOOLEAN_FLAG_MAP: Record<string, keyof CLIFormState> = {
  '-R': 'rawInput',
  '--raw-input': 'rawInput',
  '-s': 'slurp',
  '--slurp': 'slurp',
  '--tab': 'tab',
  '-a': 'ascii',
  '--ascii-output': 'ascii',
  '-S': 'sortKeys',
  '--sort-keys': 'sortKeys',
  '-e': 'exitStatus',
  '--exit-status': 'exitStatus',
  '-q': 'quiet',
  '--quiet': 'quiet',
  '--unbuffered': 'unbuffered',
  '--seq': 'seq',
};

const VALUE_SETTERS: Record<string, (s: CLIFormState, val: string) => void> = {
  '-F': (s, v) => (s.from = v),
  '--from': (s, v) => (s.from = v),
  '-T': (s, v) => (s.to = v),
  '--to': (s, v) => (s.to = v),
  '--indent': (s, v) => (s.indent = v),
};

const ENUM_FLAG_MAP: Record<string, (s: CLIFormState) => void> = {
  '-n': (s) => (s.input = 'null'),
  '--null-input': (s) => (s.input = 'null'),
  '--doc': (s) => (s.input = 'doc'),
  '--stream': (s) => (s.stream = 'stream'),
  '--stream-errors': (s) => (s.stream = 'stream-errors'),
  '-r': (s) => (s.outputMode = 'raw'),
  '--raw-output': (s) => (s.outputMode = 'raw'),
  '--raw-output0': (s) => (s.outputMode = 'raw0'),
  '-j': (s) => (s.outputMode = 'join'),
  '--join-output': (s) => (s.outputMode = 'join'),
  '-C': (s) => (s.color = 'color'),
  '--color-output': (s) => (s.color = 'color'),
  '-M': (s) => (s.color = 'mono'),
  '--monochrome-output': (s) => (s.color = 'mono'),
  '-c': (s) => (s.compact = 'compact'),
  '--compact-output': (s) => (s.compact = 'compact'),
  '--inline-output': (s) => (s.compact = 'inline'),
};

/** Best-effort reverse of {@link serializeCLIFormState}; unrecognized tokens
 * are preserved verbatim in `extra` so switching tabs never loses input. */
export const parseCLIFormState = (raw: string): CLIFormState => {
  let tokens: string[];
  try {
    tokens = parseOptions(raw);
  } catch {
    return { ...defaultCLIFormState, extra: raw };
  }

  const s: CLIFormState = { ...defaultCLIFormState };
  const extra: string[] = [];
  let i = 0;

  while (i < tokens.length) {
    const t = tokens[i]!;

    const boolKey = BOOLEAN_FLAG_MAP[t];
    if (boolKey) {
      (s[boolKey] as boolean) = true;
      i += 1;
      continue;
    }

    const enumSetter = ENUM_FLAG_MAP[t];
    if (enumSetter) {
      enumSetter(s);
      i += 1;
      continue;
    }

    const valueSetter = VALUE_SETTERS[t];
    if (valueSetter) {
      const v = tokens[i + 1];
      if (v === undefined) {
        extra.push(t);
        i += 1;
      } else {
        valueSetter(s, v);
        i += 2;
      }
      continue;
    }

    extra.push(t);
    i += 1;
  }

  s.extra = extra.map(quoteArg).join(' ');
  return s;
};

export const ANSI_COLORS: Record<number, string> = {
  30: 'ansi-black',
  31: 'ansi-red',
  32: 'ansi-green',
  33: 'ansi-yellow',
  34: 'ansi-blue',
  35: 'ansi-magenta',
  36: 'ansi-cyan',
  37: 'ansi-white',
  39: 'ansi-white',
};

export const OUTPUT_FORMAT_EXTENSIONS: Record<string, string> = {
  json: 'json',
  json5: 'json5',
  j: 'json',
  yaml: 'yaml',
  toml: 'toml',
  xml: 'xml',
  csv: 'csv',
  csvh: 'csv',
  tsv: 'tsv',
  tsvh: 'tsv',
  raw: 'txt',
  rawslurp: 'txt',
  env: 'env',
  exportenv: 'env',
  jq: 'jq',
};

export const getOutputExtension = (format: string): string => {
  return OUTPUT_FORMAT_EXTENSIONS[format] || 'txt';
};

export type AnsiSegment = {
  text: string;
  color?: string;
  bold: boolean;
  italic: boolean;
};

export const parseAnsiSegments = (text: string): AnsiSegment[] => {
  const ansi = /\x1b\[([0-9;]*)m/g;
  let color: string | undefined;
  let bold = false;
  let italic = false;
  let last = 0;
  const segments: AnsiSegment[] = [];

  for (const match of text.matchAll(ansi)) {
    if (match.index! > last) {
      segments.push({
        text: text.slice(last, match.index!),
        color,
        bold,
        italic,
      });
    }
    const codes = (match[1] || '0').split(';').map(Number);
    if (codes.includes(0)) {
      color = undefined;
      bold = false;
      italic = false;
    }
    if (codes.includes(1)) bold = true;
    if (codes.includes(3)) italic = true;
    if (codes.includes(22)) bold = false;
    if (codes.includes(23)) italic = false;
    const code = [...codes]
      .reverse()
      .find((value) => (value >= 30 && value <= 37) || value === 39);
    if (code !== undefined) color = code === 39 ? undefined : ANSI_COLORS[code];
    last = match.index! + match[0].length;
  }
  if (last < text.length) {
    segments.push({
      text: text.slice(last),
      color,
      bold,
      italic,
    });
  }
  return segments;
};

import { ansiToHtml, escapeAnsi } from '@casoon/pages-theme/ansi';
import type { ShowcaseExample } from '@casoon/pages-theme/showcase';

// Real output of `peakmark check` and `peakmark accept` for the engine module of opengrid,
// recorded with --color always. Rendered here at build time.
const files = import.meta.glob<string>('../../examples/*.ansi', {
  query: '?raw',
  import: 'default',
  eager: true,
});

const examples_ = [
  {
    slug: 'within-budget',
    title: 'Within budget and baseline',
    file: 'check.ansi',
    tags: ['check', 'size', 'memory'],
    description: 'peakmark check --details: sizes, peak memory and growth per run of one module.',
  },
  {
    slug: 'over-budget',
    title: 'Over the budget',
    file: 'fail.ansi',
    tags: ['check', 'memory', 'exit 1'],
    description: 'Peak memory over its budget: the finding names the rule and the scenario.',
  },
  {
    slug: 'no-baseline',
    title: 'No baseline yet',
    file: 'no-baseline.ansi',
    tags: ['check', 'baseline'],
    description: 'A new module warns and names the command that accepts its values.',
  },
  {
    slug: 'accept',
    title: 'Accepting a baseline',
    file: 'accept.ansi',
    tags: ['accept'],
    description: 'peakmark accept stores the measured values and logs the reason.',
  },
];

export const examples: ShowcaseExample[] = examples_.map(({ file, ...meta }) => {
  const source = files[`../../examples/${file}`];
  return {
    ...meta,
    file: `examples/${file}`,
    input: { code: escapeAnsi(source) },
    output: { html: ansiToHtml(source), kind: 'terminal' },
  };
});

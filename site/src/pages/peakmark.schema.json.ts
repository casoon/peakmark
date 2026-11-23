// The JSON Schema of peakmark.toml, served at /peakmark/peakmark.schema.json so
// that editors can point a configuration at it. The file at the repository root
// is the source; the crate's tests keep it in step with the parser.
import schema from '../../../peakmark.schema.json?raw';

export const GET = () =>
  new Response(schema, { headers: { 'Content-Type': 'application/schema+json' } });

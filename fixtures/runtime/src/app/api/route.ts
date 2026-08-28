export const runtime = 'edge';

import { nodeValue } from '../../node-only';
import nativeValue from 'node-only-kit';

export function GET() {
  return Response.json({ nodeValue, nativeValue });
}

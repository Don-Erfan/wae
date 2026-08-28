'use client';

import { serverValue } from './server';
import { nodeValue } from '../node-only';
import nativeValue from 'node-only-kit';

export const clientValue = `${serverValue}:${nodeValue}:${nativeValue}`;

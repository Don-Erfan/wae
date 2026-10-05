// format <-> currency is a module cycle (ARCH-001).
import { currency } from "./currency";

export const format = (value: number) => `${currency}${value}`;

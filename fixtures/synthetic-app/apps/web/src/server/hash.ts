import { createHash } from "node:crypto";

export const hash = (value: string) => createHash("sha256").update(value).digest("hex");

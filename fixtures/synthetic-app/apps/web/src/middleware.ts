// Edge by Next.js convention: reaches a Node builtin through a helper (RUNTIME-004).
import { hash } from "@/server/hash";

export function middleware() {
  return hash("request");
}

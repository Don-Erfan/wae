// Reaches into another feature instead of its public entrypoint (ARCH-004).
import { getSession } from "../user/model/session";

export function Cart() {
  return getSession();
}

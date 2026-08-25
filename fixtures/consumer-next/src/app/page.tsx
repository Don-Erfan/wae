import type { Cart } from "@/features/cart";
import { createCart } from "@/features/cart";

export const cart: Cart = createCart();

export async function loadCheckout() {
  return import("@/features/checkout");
}

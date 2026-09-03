import type { Cart } from "@/features/cart";
import { createCart } from "@/features/cart";

const cart: Cart = createCart();

export default async function CartPage() {
  await import("@/features/checkout");
  return <main>{cart.products.length} products</main>;
}

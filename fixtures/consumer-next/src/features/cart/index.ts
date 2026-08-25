import type { Product } from "@/entities/product";

export type Cart = { products: Product[] };
export const createCart = (): Cart => ({ products: [] });

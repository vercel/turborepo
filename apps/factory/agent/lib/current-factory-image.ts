import type { FactoryImagePointer } from "./factory-image-types";

export function requireFactoryImage(
  pointer: FactoryImagePointer | null
): FactoryImagePointer {
  if (pointer === null) {
    throw new Error(
      "No Factory image has been published. Build the shared image before creating chats."
    );
  }
  return pointer;
}

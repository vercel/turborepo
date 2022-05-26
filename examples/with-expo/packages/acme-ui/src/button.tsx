import * as React from "react";
import {
  Pressable,
  Text,
  StyleSheet,
  GestureResponderEvent,
} from "react-native";

export interface ButtonProps {
  /** Explain `color` prop here. */
  color: "primary" | "secondary";
  /** Explain `size` prop here. */
  size: "sm" | "md" | "lg";
  /** Explain `text` prop here. */
  text: string;
  /** Explain `onPress` prop here. */
  onPress?: ((event: GestureResponderEvent) => void) | null | undefined;
}

/** Explain `<Button />` component here. */
export function Button({ color, size, text, onPress }: ButtonProps) {
  return (
    <Pressable
      style={{ ...colorVariant[color], ...sizeVariant[size] }}
      onPress={onPress}
    >
      <Text>{text}</Text>
    </Pressable>
  );
}

const colorVariant = StyleSheet.create({
  primary: {
    backgroundColor: "pink",
  },
  secondary: {
    backgroundColor: "yellow",
  },
});

const sizeVariant = StyleSheet.create({
  sm: {
    padding: 10,
  },
  md: {
    padding: 20,
  },
  lg: {
    padding: 30,
  },
});

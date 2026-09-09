import type { ComponentProps } from "react";
import { Button as UiButton } from "@/components/ui/button";
import { cn } from "@/lib/cn";

/**
 * Strips shadcn's 36px height, `rounded-md`, focus ring and `shadow-xs`, and
 * replaces the variant palette with Mnemos tokens. Feature code imports from
 * here, never from `@/components/ui/button`.
 */
export type ButtonVariant = "primary" | "secondary" | "ghost" | "destructive";
export type ButtonSize = "default" | "lg" | "icon";

type ButtonProps = Omit<ComponentProps<typeof UiButton>, "variant" | "size"> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
};

const variantClasses: Record<ButtonVariant, string> = {
  primary: "bg-accent-primary text-inverse hover:bg-accent-primary-hover",
  secondary: "bg-elevated text-primary border border-strong hover:bg-hover",
  ghost: "bg-transparent text-primary hover:bg-hover",
  destructive: "bg-danger text-inverse hover:brightness-110",
};

const sizeClasses: Record<ButtonSize, string> = {
  default: "h-8 px-3",
  lg: "h-10 px-4",
  icon: "size-8 p-0",
};

export function Button({ variant = "primary", size = "default", className, ...rest }: ButtonProps) {
  return (
    <UiButton
      className={cn(
        "rounded-sm font-medium text-sm shadow-none transition-colors",
        "motion-quick focus-visible:border-0 focus-visible:ring-0",
        "disabled:opacity-40",
        sizeClasses[size],
        variantClasses[variant],
        className,
      )}
      {...rest}
    />
  );
}

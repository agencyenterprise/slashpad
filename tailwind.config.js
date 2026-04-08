/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        surface: {
          0: "#0a0a0b",
          1: "#111113",
          2: "#18181b",
          3: "#222225",
        },
        accent: {
          DEFAULT: "#c4a1ff",
          dim: "#8b6cc1",
          bright: "#dbc5ff",
        },
        muted: "#6b6b76",
        success: "#4ade80",
        warning: "#fbbf24",
        danger: "#f87171",
      },
      fontFamily: {
        mono: ['"Berkeley Mono"', '"JetBrains Mono"', '"SF Mono"', "Menlo", "monospace"],
        sans: ['"Inter"', "system-ui", "sans-serif"],
      },
      animation: {
        "fade-in": "fadeIn 0.15s ease-out",
        "slide-down": "slideDown 0.2s ease-out",
        "pulse-subtle": "pulseSubtle 2s ease-in-out infinite",
      },
      keyframes: {
        fadeIn: {
          "0%": { opacity: "0" },
          "100%": { opacity: "1" },
        },
        slideDown: {
          "0%": { opacity: "0", transform: "translateY(-4px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        pulseSubtle: {
          "0%, 100%": { opacity: "1" },
          "50%": { opacity: "0.6" },
        },
      },
    },
  },
  plugins: [],
};

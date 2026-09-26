import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider, createTheme } from "@mui/material";
import App from "./App";

const theme = createTheme({
  palette: {
    mode: "light",
    primary: { main: "#16754b", dark: "#0e5535", contrastText: "#ffffff" },
    secondary: { main: "#5b675f" },
    background: { default: "#f3f6f4", paper: "#ffffff" },
    success: { main: "#16754b" },
    warning: { main: "#a15c00" },
    error: { main: "#b3261e" },
  },
  shape: { borderRadius: 12 },
  typography: {
    fontFamily: 'Inter, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
    h4: { fontWeight: 750, letterSpacing: "-0.03em" },
    h5: { fontWeight: 700, letterSpacing: "-0.02em" },
    h6: { fontWeight: 700 },
    button: { fontWeight: 700, textTransform: "none" },
  },
  components: {
    MuiButton: { defaultProps: { disableElevation: true }, styleOverrides: { root: { minHeight: 40 } } },
    MuiCard: { styleOverrides: { root: { border: "1px solid #dce5df", boxShadow: "0 2px 8px rgba(18, 55, 35, 0.05)" } } },
    MuiListItemButton: { styleOverrides: { root: { borderRadius: 10 } } },
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <App />
    </ThemeProvider>
  </StrictMode>,
);

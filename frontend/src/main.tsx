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
    MuiButton: { defaultProps: { disableElevation: true }, styleOverrides: { root: { minHeight: 40, borderRadius: 10, paddingInline: 16, transition: "background-color 160ms ease, border-color 160ms ease, color 160ms ease, transform 160ms ease", "&:active": { transform: "translateY(1px)" } } } },
    MuiCard: { styleOverrides: { root: { border: "1px solid #dce5df", boxShadow: "0 2px 8px rgba(18, 55, 35, 0.05)", transition: "border-color 160ms ease, background-color 160ms ease, box-shadow 160ms ease" } } },
    MuiChip: { styleOverrides: { root: { fontWeight: 650 }, sizeSmall: { height: 26 } } },
    MuiDialog: { styleOverrides: { paper: { borderRadius: 16 } } },
    MuiDialogActions: { styleOverrides: { root: { padding: "16px 24px 20px", gap: 8 } } },
    MuiListItemButton: { styleOverrides: { root: { borderRadius: 10, transition: "background-color 160ms ease, color 160ms ease" } } },
    MuiToggleButton: { styleOverrides: { root: { minHeight: 36, paddingInline: 10, fontWeight: 650, textTransform: "none", whiteSpace: "nowrap", "&.Mui-selected": { color: "#ffffff", backgroundColor: "#16754b", "&:hover": { backgroundColor: "#0e5535" } } } } },
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

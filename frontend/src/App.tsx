import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  AppBar,
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  Drawer,
  Fade,
  FormControl,
  FormControlLabel,
  InputLabel,
  LinearProgress,
  List,
  ListItem,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  MenuItem,
  Select,
  Snackbar,
  Stack,
  Switch,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Toolbar,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  AddRounded,
  AndroidRounded,
  AspectRatioRounded,
  CheckCircleRounded,
  ChevronRightRounded,
  CodeRounded,
  ContentCopyRounded,
  DeleteOutlineRounded,
  ExpandMoreRounded,
  HelpOutlineRounded,
  KeyboardRounded,
  MemoryRounded,
  MonitorHeartRounded,
  PhoneAndroidRounded,
  PlayArrowRounded,
  RefreshRounded,
  SettingsRounded,
  StopRounded,
  TerminalRounded,
  TuneRounded,
  VolumeOffRounded,
  VisibilityOffRounded,
  WarningAmberRounded,
} from "@mui/icons-material";

type Page = "androids" | "monitor" | "logs" | "settings";
type PendingAction = "starting" | "stopping";
type Speed = "Efficient" | "LeanGaming" | "Balanced" | "Fast";
type Picture = "Compact" | "Phone" | "Sharp";

type AndroidProfile = {
  name: string;
  device_name: string;
  api_level: number | null;
  resolution: string | null;
  running_serial: string | null;
};

type ProfileOptions = {
  speed: Speed;
  picture: Picture;
  window: "Remember" | "Portrait" | "Landscape";
  cores: number;
  memory_mb: number;
  dpi: number;
  adb_port: number | null;
  gpu_mode: string;
  cold_boot: boolean;
  host_keyboard: boolean;
  device_frame: boolean;
  rectangular_display: boolean;
  mute_audio: boolean;
  window_scale: number | null;
  headless_automation: boolean;
  disable_vulkan: boolean;
  refresh_rate_hz: number;
  suspend_store_during_automation: boolean;
  host_memory_policy: boolean;
  memory_high_mb: number;
  memory_max_mb: number;
  memory_swap_max_mb: number;
};

type DeviceHostStats = {
  profile_name: string;
  serial: string;
  pid: number | null;
  rss_bytes: number;
  pss_bytes: number;
  vm_swap_bytes: number;
  scope_name: string | null;
  memory_current_bytes: number | null;
  memory_high_bytes: number | null;
  memory_max_bytes: number | null;
  memory_swap_current_bytes: number | null;
  memory_events: Record<string, number>;
  pressure_some_avg10: number | null;
  pressure_full_avg10: number | null;
  warning: string | null;
};

type AppState = {
  profiles: AndroidProfile[];
  profile_options: Record<string, ProfileOptions>;
  starting_profiles: string[];
  queued_profiles: string[];
  system_images: { package_id: string; label: string }[];
  config: { android_sdk_path: string; jdk_path: string };
  health: { emulator: boolean; adb: boolean; avd_manager: boolean; kvm: boolean; java: boolean; systemd_run: boolean };
  stats: { cpu_percent: number; memory_used_bytes: number; memory_total_bytes: number };
  device_stats: DeviceHostStats[];
  logs: { at: number; level: string; message: string }[];
};

const drawerWidth = 244;
const defaultOptions: ProfileOptions = {
  speed: "LeanGaming",
  picture: "Phone",
  window: "Remember",
  cores: 2,
  memory_mb: 4096,
  dpi: 240,
  adb_port: null,
  gpu_mode: "host",
  cold_boot: false,
  host_keyboard: true,
  device_frame: false,
  rectangular_display: true,
  mute_audio: true,
  window_scale: 0.55,
  headless_automation: false,
  disable_vulkan: false,
  refresh_rate_hz: 30,
  suspend_store_during_automation: true,
  host_memory_policy: true,
  memory_high_mb: 6656,
  memory_max_mb: 7168,
  memory_swap_max_mb: 1024,
};

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { "Content-Type": "application/json", ...init?.headers },
  });
  const body = await response.json().catch(() => ({ message: "Unexpected response" }));
  if (!response.ok) throw new Error(body.message || `Request failed (${response.status})`);
  return body as T;
}

function statusLabel(profile: AndroidProfile, pending?: PendingAction, phase?: "starting" | "queued") {
  if (phase === "queued") return "Queued";
  if (phase === "starting") return "Starting…";
  if (pending === "starting") return "Starting…";
  if (pending === "stopping") return "Stopping…";
  return profile.running_serial ? "Running" : "Stopped";
}

function runActionLabel(profile: AndroidProfile, pending?: PendingAction, phase?: "starting" | "queued") {
  if (phase === "queued") return "Cancel";
  if (phase === "starting") return "Starting…";
  if (pending === "starting") return "Starting…";
  if (pending === "stopping") return "Stopping…";
  return profile.running_serial ? "Stop" : "Start";
}

function displayProfileName(name: string) {
  const automatic = /^Device_(\d+)$/.exec(name);
  return automatic ? `Device ${automatic[1]}` : name;
}

function nextDeviceName(profiles: AndroidProfile[]) {
  const used = new Set(profiles.map((profile) => profile.name));
  let number = 1;
  while (used.has(`Device_${number}`)) number += 1;
  return `Device ${number}`;
}

function internalProfileName(name: string) {
  return name.trim().replace(/\s+/g, "_");
}

function gib(bytes: number) {
  return (bytes / 1024 / 1024 / 1024).toFixed(1);
}

function mib(bytes: number | null) {
  return bytes === null ? "Unavailable" : `${Math.round(bytes / 1024 / 1024)} MB`;
}

function resolutionLabel(picture: Picture) {
  return picture === "Compact" ? "540 × 960" : picture === "Sharp" ? "1080 × 1920" : "720 × 1280";
}

function relativeTime(seconds: number) {
  const ago = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  if (ago < 5) return "now";
  if (ago < 60) return `${ago}s ago`;
  if (ago < 3600) return `${Math.floor(ago / 60)}m ago`;
  return `${Math.floor(ago / 3600)}h ago`;
}

export default function App() {
  const [page, setPage] = useState<Page>("androids");
  const [state, setState] = useState<AppState | null>(null);
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [pending, setPending] = useState<Record<string, PendingAction>>({});
  const [createOpen, setCreateOpen] = useState(false);
  const [deleteName, setDeleteName] = useState<string | null>(null);
  const [cloneSource, setCloneSource] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ message: string; error: boolean } | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async (showError = false) => {
    try {
      const next = await request<AppState>("/api/state");
      setState(next);
      setSelectedName((current) =>
        current && next.profiles.some((profile) => profile.name === current)
          ? current
          : next.profiles[0]?.name ?? null,
      );
      setPending((current) => {
        const updated = { ...current };
        for (const [name, action] of Object.entries(updated)) {
          const profile = next.profiles.find((item) => item.name === name);
          const activeStart = next.starting_profiles.includes(name) || next.queued_profiles.includes(name);
          if (!profile || (action === "starting" && (profile.running_serial || activeStart)) || (action === "stopping" && !profile.running_serial && !activeStart)) {
            delete updated[name];
          }
        }
        return updated;
      });
    } catch (error) {
      if (showError) setNotice({ message: (error as Error).message, error: true });
    }
  }, []);

  useEffect(() => {
    void refresh(true);
    const timer = window.setInterval(() => void refresh(), 2000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const perform = async (path: string, init: RequestInit, success?: () => void) => {
    setBusy(true);
    try {
      const result = await request<{ message: string }>(path, init);
      setNotice({ message: result.message, error: false });
      success?.();
      await refresh(true);
    } catch (error) {
      setNotice({ message: (error as Error).message, error: true });
      throw error;
    } finally {
      setBusy(false);
    }
  };

  const selected = state?.profiles.find((profile) => profile.name === selectedName) ?? null;
  const ready = Boolean(state?.health.emulator && state.health.adb && state.health.kvm);

  const nav: { id: Page; label: string; icon: React.ReactNode }[] = [
    { id: "androids", label: "My Androids", icon: <PhoneAndroidRounded /> },
    { id: "monitor", label: "Monitor", icon: <MonitorHeartRounded /> },
    { id: "logs", label: "Activity", icon: <TerminalRounded /> },
    { id: "settings", label: "Settings", icon: <SettingsRounded /> },
  ];

  return (
    <Box sx={{ display: "flex", height: "100vh", overflow: "hidden" }}>
      <Drawer
        variant="permanent"
        sx={{ width: drawerWidth, flexShrink: 0, "& .MuiDrawer-paper": { width: drawerWidth, boxSizing: "border-box", bgcolor: "#111714", color: "#eaf2ed", border: 0 } }}
      >
        <Toolbar sx={{ gap: 1.5, minHeight: "72px !important" }}>
          <AndroidRounded color="success" fontSize="large" />
          <Box>
            <Typography variant="h6" lineHeight={1}>EmuMi</Typography>
            <Typography variant="caption" color="#9fb0a6">Androids, simply.</Typography>
          </Box>
        </Toolbar>
        <List sx={{ px: 1.5, py: 2 }}>
          {nav.map((item) => (
            <ListItem key={item.id} disablePadding sx={{ mb: 0.5 }}>
              <ListItemButton selected={page === item.id} onClick={() => setPage(item.id)} sx={{ "&.Mui-selected": { bgcolor: "#244333", color: "#a6e6c2" }, "&.Mui-selected:hover": { bgcolor: "#2a4c39" } }}>
                <ListItemIcon sx={{ minWidth: 40, color: "inherit" }}>{item.icon}</ListItemIcon>
                <ListItemText primary={item.label} primaryTypographyProps={{ fontWeight: 650 }} />
              </ListItemButton>
            </ListItem>
          ))}
        </List>
        <Box sx={{ mt: "auto", p: 2 }}>
          <Alert severity={ready ? "success" : "warning"} variant="outlined" sx={{ color: "inherit", borderColor: ready ? "#315d44" : "#6d5530", "& .MuiAlert-icon": { color: ready ? "#6ed49a" : "#edb85e" } }}>
            <Typography variant="subtitle2">{ready ? "System ready" : "Setup needed"}</Typography>
            <Typography variant="caption">{ready ? "SDK, ADB and KVM found" : "Check Settings"}</Typography>
          </Alert>
        </Box>
      </Drawer>

      <Box component="main" sx={{ display: "flex", flexDirection: "column", flexGrow: 1, minWidth: 0, minHeight: 0, overflow: "hidden" }}>
        <AppBar position="static" color="inherit" elevation={0} sx={{ borderBottom: "1px solid", borderColor: "divider", flexShrink: 0 }}>
          <Toolbar sx={{ minHeight: "72px !important", gap: 2 }}>
            <Box sx={{ flexGrow: 1 }}>
              <Typography variant="h6">{nav.find((item) => item.id === page)?.label}</Typography>
              <Typography variant="body2" color="text.secondary">
                {page === "androids" && "Create, tune and launch virtual Android devices"}
                {page === "monitor" && "A clear view of what is running"}
                {page === "logs" && "Recent emulator manager activity"}
                {page === "settings" && "Tools, paths and system readiness"}
              </Typography>
            </Box>
            <Tooltip title="Refresh now"><span><Button variant="outlined" startIcon={busy ? <CircularProgress size={17} /> : <RefreshRounded />} onClick={() => void refresh(true)} disabled={!state || busy}>Refresh</Button></span></Tooltip>
            {page === "androids" && <Button variant="contained" startIcon={<AddRounded />} disabled={busy} onClick={() => setCreateOpen(true)}>New Android</Button>}
          </Toolbar>
        </AppBar>

        <Fade in key={page} timeout={180}>
        <Box sx={{ p: { xs: 2, lg: 3 }, width: "100%", maxWidth: 1440, mx: "auto", flexGrow: 1, minHeight: 0, overflowY: page === "androids" ? "hidden" : "auto" }}>
          {!state ? (
            <Stack alignItems="center" justifyContent="center" spacing={2} minHeight="60vh"><CircularProgress /><Typography color="text.secondary">Reading your Android setup…</Typography></Stack>
          ) : page === "androids" ? (
            <AndroidsPage
              state={state}
              selected={selected}
              selectedName={selectedName}
              setSelectedName={setSelectedName}
              pending={pending}
              setPending={setPending}
              perform={perform}
              setDeleteName={setDeleteName}
              setCloneSource={setCloneSource}
              busy={busy}
            />
          ) : page === "monitor" ? (
            <MonitorPage state={state} />
          ) : page === "logs" ? (
            <LogsPage state={state} />
          ) : (
            <SettingsPage state={state} perform={perform} busy={busy} />
          )}
        </Box>
        </Fade>
      </Box>

      {state && <CreateDialog open={createOpen} onClose={() => setCreateOpen(false)} state={state} busy={busy} perform={perform} onCreated={setSelectedName} />}
      <CloneDialog source={cloneSource} profiles={state?.profiles ?? []} onClose={() => setCloneSource(null)} busy={busy} perform={perform} onCreated={setSelectedName} />
      <Dialog open={Boolean(deleteName)} onClose={busy ? undefined : () => setDeleteName(null)} maxWidth="xs" fullWidth>
        <DialogTitle>Delete {deleteName ? displayProfileName(deleteName) : "Android"}?</DialogTitle>
        <DialogContent><Typography color="text.secondary">Its installed apps, files and emulator data will be permanently removed.</Typography></DialogContent>
        <DialogActions>
          <Button onClick={() => setDeleteName(null)} disabled={busy}>Cancel</Button>
          <Button color="error" variant="contained" startIcon={busy ? <CircularProgress size={18} color="inherit" /> : <DeleteOutlineRounded />} disabled={busy} onClick={() => deleteName && void perform(`/api/profiles/${encodeURIComponent(deleteName)}`, { method: "DELETE" }, () => setDeleteName(null))}>{busy ? "Deleting…" : "Delete Android"}</Button>
        </DialogActions>
      </Dialog>
      <Snackbar open={Boolean(notice)} autoHideDuration={3500} onClose={() => setNotice(null)} anchorOrigin={{ vertical: "bottom", horizontal: "center" }}>
        <Alert severity={notice?.error ? "error" : "success"} variant="filled" onClose={() => setNotice(null)}>{notice?.message}</Alert>
      </Snackbar>
    </Box>
  );
}

type Perform = (path: string, init: RequestInit, success?: () => void) => Promise<void>;

function AndroidsPage({ state, selected, selectedName, setSelectedName, pending, setPending, perform, setDeleteName, setCloneSource, busy }: {
  state: AppState;
  selected: AndroidProfile | null;
  selectedName: string | null;
  setSelectedName: (name: string) => void;
  pending: Record<string, PendingAction>;
  setPending: React.Dispatch<React.SetStateAction<Record<string, PendingAction>>>;
  perform: Perform;
  setDeleteName: (name: string) => void;
  setCloneSource: (name: string) => void;
  busy: boolean;
}) {
  return (
    <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", lg: "minmax(260px, 340px) minmax(0, 1fr)" }, gridTemplateRows: { xs: "minmax(180px, auto) minmax(0, 1fr)", lg: "minmax(0, 1fr)" }, gap: 3, height: "100%", minHeight: 0 }}>
      <Stack spacing={1.5} sx={{ minHeight: 0, overflowY: "auto", pr: 0.75 }}>
        <Typography variant="overline" color="text.secondary">Your devices · {state.profiles.length}</Typography>
        {state.profiles.length === 0 ? (
          <EmptyState icon={<PhoneAndroidRounded fontSize="large" />} title="No Androids yet" detail="Use New Android to create your first profile." />
        ) : state.profiles.map((profile) => {
          const running = Boolean(profile.running_serial);
          const phase = state.queued_profiles.includes(profile.name) ? "queued" : state.starting_profiles.includes(profile.name) ? "starting" : undefined;
          const adbSlot = state.profile_options[profile.name]?.adb_port;
          return (
            <Card key={profile.name} variant="outlined" sx={{ borderColor: selectedName === profile.name ? "primary.main" : undefined, bgcolor: selectedName === profile.name ? "rgba(22, 117, 75, 0.045)" : "background.paper" }}>
              <ListItemButton selected={selectedName === profile.name} onClick={() => setSelectedName(profile.name)} sx={{ py: 1.5 }}>
                <ListItemIcon><PhoneAndroidRounded color={running ? "success" : "action"} /></ListItemIcon>
                <ListItemText primary={displayProfileName(profile.name)} secondary={`${profile.device_name || "Android device"}${profile.api_level ? ` · API ${profile.api_level}` : ""}${adbSlot ? ` · ADB ${adbSlot}` : ""}`} primaryTypographyProps={{ fontWeight: 700 }} />
                <Chip size="small" color={pending[profile.name] || phase ? "warning" : running ? "success" : "default"} label={statusLabel(profile, pending[profile.name], phase)} />
              </ListItemButton>
            </Card>
          );
        })}
      </Stack>
      {selected ? (
        <Fade in key={selected.name} timeout={160}><Box sx={{ minHeight: 0, overflowY: "auto", pr: 0.75 }}><ProfileDetail profile={selected} options={state.profile_options[selected.name] ?? defaultOptions} pending={pending[selected.name]} phase={state.queued_profiles.includes(selected.name) ? "queued" : state.starting_profiles.includes(selected.name) ? "starting" : undefined} setPending={setPending} perform={perform} setDeleteName={setDeleteName} setCloneSource={setCloneSource} busy={busy} /></Box></Fade>
      ) : (
        <EmptyState icon={<AndroidRounded fontSize="large" />} title="Choose an Android" detail="Its controls and settings will appear here." />
      )}
    </Box>
  );
}

function ProfileDetail({ profile, options, pending, phase, setPending, perform, setDeleteName, setCloneSource, busy }: {
  profile: AndroidProfile;
  options: ProfileOptions;
  pending?: PendingAction;
  phase?: "starting" | "queued";
  setPending: React.Dispatch<React.SetStateAction<Record<string, PendingAction>>>;
  perform: Perform;
  setDeleteName: (name: string) => void;
  setCloneSource: (name: string) => void;
  busy: boolean;
}) {
  const [draft, setDraft] = useState(options);
  const [saving, setSaving] = useState(false);
  useEffect(() => setDraft(options), [profile.name]);
  const running = Boolean(profile.running_serial);
  const dirty = useMemo(() => JSON.stringify(draft) !== JSON.stringify(options), [draft, options]);

  const toggleRun = async () => {
    const shouldStop = running || phase === "queued";
    const action: PendingAction = shouldStop ? "stopping" : "starting";
    setPending((value) => ({ ...value, [profile.name]: action }));
    try {
      await perform(`/api/profiles/${encodeURIComponent(profile.name)}/${shouldStop ? "stop" : "start"}`, { method: "POST" });
    } catch {
      setPending((value) => { const next = { ...value }; delete next[profile.name]; return next; });
    }
  };

  const chooseSpeed = (speed: Speed) => {
    const resources = { Efficient: [2, 2048], LeanGaming: [2, 4096], Balanced: [4, 4096], Fast: [8, 8192] } as const;
    setDraft((value) => ({
      ...value,
      speed,
      cores: resources[speed][0],
      memory_mb: resources[speed][1],
      ...(speed === "LeanGaming" ? {
        picture: "Phone" as Picture,
        dpi: 240,
        gpu_mode: value.gpu_mode === "host-intel" ? "host-intel" : "host",
        mute_audio: true,
        rectangular_display: true,
        refresh_rate_hz: 30,
        suspend_store_during_automation: true,
        host_memory_policy: true,
        memory_high_mb: 6656,
        memory_max_mb: 7168,
        memory_swap_max_mb: 1024,
      } : {}),
    }));
  };

  const save = async () => {
    setSaving(true);
    try {
      await perform(`/api/profiles/${encodeURIComponent(profile.name)}/settings`, { method: "POST", body: JSON.stringify(draft) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Stack spacing={3}>
      <Card><CardContent>
        <Stack spacing={2}>
          <Stack direction="row" gap={2} alignItems="center">
            <Box sx={{ width: 56, height: 56, borderRadius: 3, bgcolor: "primary.main", color: "primary.contrastText", display: "grid", flexShrink: 0, placeItems: "center" }}><AndroidRounded fontSize="large" /></Box>
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="h5" noWrap>{displayProfileName(profile.name)}</Typography>
              <Typography color="text.secondary">{profile.device_name || "Android device"}{profile.api_level ? ` · Android API ${profile.api_level}` : ""}</Typography>
              <Stack direction="row" gap={1} mt={1} flexWrap="wrap">
                <Chip size="small" color={pending || phase ? "warning" : running ? "success" : "default"} label={statusLabel(profile, pending, phase)} />
                <Chip size="small" variant="outlined" label={`${resolutionLabel(options.picture)} configured`} />
                <Chip size="small" variant="outlined" label={options.adb_port ? `ADB ${options.adb_port}` : "ADB automatic"} />
                {profile.running_serial && <Chip size="small" variant="outlined" label={profile.running_serial} />}
              </Stack>
            </Box>
          </Stack>
          <Divider />
          <Stack direction="row" gap={1} justifyContent="flex-end">
            <Tooltip title={running ? "Stop this Android before cloning it" : "Copy apps, accounts and data"}><span><Button sx={{ minWidth: 104 }} variant="outlined" startIcon={<ContentCopyRounded />} disabled={running || Boolean(pending) || busy} onClick={() => setCloneSource(profile.name)}>Clone</Button></span></Tooltip>
            <Tooltip title={running ? "Stop this Android before deleting it" : "Delete this Android"}><span><Button sx={{ minWidth: 104 }} variant="outlined" color="error" startIcon={<DeleteOutlineRounded />} disabled={running || Boolean(pending) || busy} onClick={() => setDeleteName(profile.name)}>Delete</Button></span></Tooltip>
            <Button sx={{ minWidth: 112 }} variant="contained" color={running || phase === "queued" ? "error" : "primary"} startIcon={pending || phase === "starting" ? <CircularProgress size={18} color="inherit" /> : running || phase === "queued" ? <StopRounded /> : <PlayArrowRounded />} disabled={Boolean(pending) || phase === "starting" || busy} onClick={() => void toggleRun()}>{runActionLabel(profile, pending, phase)}</Button>
          </Stack>
        </Stack>
      </CardContent></Card>

      <Card><CardContent>
        <Stack direction="row" alignItems="center" mb={2}>
          <Box sx={{ flexGrow: 1 }}><Typography variant="h6">Quick setup</Typography><Typography variant="body2" color="text.secondary">The useful controls, without emulator jargon.</Typography></Box>
          <Button sx={{ minWidth: 132 }} variant={dirty ? "contained" : "outlined"} startIcon={saving ? <CircularProgress size={18} color="inherit" /> : !dirty ? <CheckCircleRounded /> : undefined} disabled={!dirty || saving || (busy && !saving)} onClick={() => void save()}>{saving ? "Saving…" : dirty ? "Save changes" : "Saved"}</Button>
        </Stack>
        <Divider />
        <SettingRow icon={<MemoryRounded />} title="Performance" description="Choose how much of this computer Android may use">
          <ToggleButtonGroup exclusive value={draft.speed} size="small" onChange={(_, value: Speed | null) => value && chooseSpeed(value)}>
            <ToggleButton value="Efficient">Efficient</ToggleButton><ToggleButton value="LeanGaming">Lean gaming</ToggleButton><ToggleButton value="Balanced">Balanced</ToggleButton><ToggleButton value="Fast">Fast</ToggleButton>
          </ToggleButtonGroup>
        </SettingRow>
        <Divider />
        <SettingRow icon={<AspectRatioRounded />} title="Resolution" description="720 × 1280 is the default for Frostguard">
          <ToggleButtonGroup exclusive value={draft.picture} size="small" onChange={(_, value: Picture | null) => value && setDraft({ ...draft, picture: value })}>
            <ToggleButton value="Compact">540 × 960</ToggleButton><ToggleButton value="Phone">720 × 1280</ToggleButton><ToggleButton value="Sharp">1080 × 1920</ToggleButton>
          </ToggleButtonGroup>
        </SettingRow>
        <Divider />
        <SettingRow icon={<MonitorHeartRounded />} title="Window size" description="Changes how large Android appears on your desktop">
          <ToggleButtonGroup exclusive value={draft.window_scale ?? "remember"} size="small" onChange={(_, value: number | "remember" | null) => value !== null && setDraft({ ...draft, window_scale: value === "remember" ? null : value })}>
            <ToggleButton value="remember">Remember</ToggleButton><ToggleButton value={0.45}>Compact</ToggleButton><ToggleButton value={0.55}>Comfortable</ToggleButton><ToggleButton value={0.7}>Large</ToggleButton>
          </ToggleButtonGroup>
        </SettingRow>
        <Divider />
        <SettingRow icon={<TuneRounded />} title="Android UI size" description="Controls the size of text, buttons and apps inside Android">
          <ToggleButtonGroup exclusive value={draft.dpi} size="small" onChange={(_, value: number | null) => value && setDraft({ ...draft, dpi: value })}>
            <ToggleButton value={240}>Smaller</ToggleButton><ToggleButton value={280}>Normal</ToggleButton><ToggleButton value={320}>Larger</ToggleButton>
          </ToggleButtonGroup>
        </SettingRow>
        <Divider />
        <SettingRow icon={<KeyboardRounded />} title="Use computer keyboard" description="Send Linux keyboard input into Android">
          <Switch checked={draft.host_keyboard} onChange={(event) => setDraft({ ...draft, host_keyboard: event.target.checked })} />
        </SettingRow>
        <Divider />
        <SettingRow icon={<PhoneAndroidRounded />} title="Show device frame" description="Keep off for a clean, easier-to-resize window">
          <Switch checked={draft.device_frame} onChange={(event) => setDraft({ ...draft, device_frame: event.target.checked })} />
        </SettingRow>
        <Divider />
        <SettingRow icon={<AspectRatioRounded />} title="Rectangular app screen" description="Removes camera cutouts so automation coordinates line up">
          <Switch checked={draft.rectangular_display} onChange={(event) => setDraft({ ...draft, rectangular_display: event.target.checked })} />
        </SettingRow>
        <Divider />
        <SettingRow icon={<VolumeOffRounded />} title="Mute Android audio" description="Launch this Android without sound output">
          <Switch checked={draft.mute_audio} onChange={(event) => setDraft({ ...draft, mute_audio: event.target.checked })} />
        </SettingRow>
        <Divider />
        <SettingRow icon={<VisibilityOffRounded />} title="Headless automation" description="Run without an emulator window; ADB screenshots and Frostguard input remain available">
          <Switch checked={draft.headless_automation} onChange={(event) => setDraft({ ...draft, headless_automation: event.target.checked })} />
        </SettingRow>
        {draft.headless_automation && <Alert severity="info" sx={{ mt: 1 }}>Also disables both cameras and the boot animation. Verify Frostguard screenshots, OCR and taps at 720 × 1280 before relying on it unattended.</Alert>}
        <Divider />
        <SettingRow icon={<MonitorHeartRounded />} title="Automation frame rate" description="30 Hz cuts continuous rendering work; use 60 Hz only for interactive play">
          <ToggleButtonGroup exclusive value={draft.refresh_rate_hz} size="small" onChange={(_, value: number | null) => value && setDraft({ ...draft, refresh_rate_hz: value })}>
            <ToggleButton value={30}>30 Hz</ToggleButton><ToggleButton value={45}>45 Hz</ToggleButton><ToggleButton value={60}>60 Hz</ToggleButton>
          </ToggleButtonGroup>
        </SettingRow>
        <Divider />
        <SettingRow icon={<RefreshRounded />} title="Pause app updates during automation" description="Keeps Play Store installed but prevents restores and updates from consuming CPU while bots run">
          <Switch checked={draft.suspend_store_during_automation} onChange={(event) => setDraft({ ...draft, suspend_store_during_automation: event.target.checked })} />
        </SettingRow>
        {draft.speed === "LeanGaming" && <Alert severity="info" sx={{ mt: 1 }}>Lean Gaming always starts Android from a clean boot so native games do not inherit a broken saved-memory state. Installed apps, game data and accounts are preserved.</Alert>}
        <Accordion disableGutters elevation={0} sx={{ mt: 1, "&:before": { display: "none" } }}>
          <AccordionSummary expandIcon={<ExpandMoreRounded />}><TuneRounded sx={{ mr: 2 }} /><Box><Typography fontWeight={700}>Advanced settings</Typography><Typography variant="body2" color="text.secondary">CPU, memory, graphics, ports and boot behavior</Typography></Box></AccordionSummary>
          <AccordionDetails>
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", sm: "1fr 1fr" }, gap: 2 }}>
              <TextField label="CPU cores" type="number" value={draft.cores} onChange={(event) => setDraft({ ...draft, cores: Number(event.target.value) })} slotProps={{ htmlInput: { min: 1, max: 16 } }} />
              <TextField label="Memory (MB)" type="number" value={draft.memory_mb} onChange={(event) => setDraft({ ...draft, memory_mb: Number(event.target.value) })} slotProps={{ htmlInput: { min: 512, max: 16384, step: 256 } }} />
              <TextField label="Refresh rate (Hz)" type="number" value={draft.refresh_rate_hz} onChange={(event) => setDraft({ ...draft, refresh_rate_hz: Number(event.target.value) })} slotProps={{ htmlInput: { min: 15, max: 120, step: 1 } }} />
              <FormControl><InputLabel>Graphics</InputLabel><Select label="Graphics" value={draft.gpu_mode} onChange={(event) => setDraft({ ...draft, gpu_mode: event.target.value })}><MenuItem value="auto">Automatic</MenuItem><MenuItem value="host">Hardware</MenuItem><MenuItem value="host-intel">Hardware · Intel (Mesa)</MenuItem><MenuItem value="swiftshader_indirect">Software</MenuItem></Select></FormControl>
              <TextField label="ADB slot" type="number" value={draft.adb_port ?? ""} placeholder="Assigned automatically" helperText="Stable slots use 5554, 5556, 5558…" onChange={(event) => setDraft({ ...draft, adb_port: event.target.value ? Number(event.target.value) : null })} slotProps={{ htmlInput: { min: 5554, max: 5682, step: 2 } }} />
              <FormControlLabel control={<Switch checked={draft.speed === "LeanGaming" || draft.cold_boot} disabled={draft.speed === "LeanGaming"} onChange={(event) => setDraft({ ...draft, cold_boot: event.target.checked })} />} label={draft.speed === "LeanGaming" ? "Clean boot always on" : "Cold boot next time"} />
              <FormControlLabel control={<Switch checked={draft.disable_vulkan} onChange={(event) => setDraft({ ...draft, disable_vulkan: event.target.checked })} />} label="Experimental: disable Vulkan" />
            </Box>
            <Divider sx={{ my: 2 }} />
            <FormControlLabel control={<Switch checked={draft.host_memory_policy} onChange={(event) => setDraft({ ...draft, host_memory_policy: event.target.checked })} />} label="Protect the Linux host with per-emulator resource controls" />
            {draft.host_memory_policy && <Stack spacing={2} mt={2}>
              <Alert severity="info">EmuMi gives this Android lower CPU and disk priority under contention, starts memory reclaim at MemoryHigh, and treats MemoryMax as a last-resort ceiling. Android still receives its configured guest memory.</Alert>
              <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", sm: "repeat(3, 1fr)" }, gap: 2 }}>
                <TextField label="MemoryHigh (MB)" type="number" value={draft.memory_high_mb} helperText="Reclaim begins here" onChange={(event) => setDraft({ ...draft, memory_high_mb: Number(event.target.value) })} slotProps={{ htmlInput: { min: 4096, step: 256 } }} />
                <TextField label="MemoryMax (MB)" type="number" value={draft.memory_max_mb} helperText="Last-resort ceiling" onChange={(event) => setDraft({ ...draft, memory_max_mb: Number(event.target.value) })} slotProps={{ htmlInput: { min: 4096, step: 256 } }} />
                <TextField label="SwapMax (MB)" type="number" value={draft.memory_swap_max_mb} helperText="Host swap allowance" onChange={(event) => setDraft({ ...draft, memory_swap_max_mb: Number(event.target.value) })} slotProps={{ htmlInput: { min: 0, step: 256 } }} />
              </Box>
            </Stack>}
          </AccordionDetails>
        </Accordion>
      </CardContent></Card>
    </Stack>
  );
}

function SettingRow({ icon, title, description, children }: { icon: React.ReactNode; title: string; description: string; children: React.ReactNode }) {
  return <Stack direction={{ xs: "column", sm: "row" }} gap={2} alignItems={{ sm: "center" }} py={2.25}><Box color="primary.main" sx={{ display: "flex" }}>{icon}</Box><Box sx={{ flexGrow: 1 }}><Typography fontWeight={700}>{title}</Typography><Typography variant="body2" color="text.secondary">{description}</Typography></Box>{children}</Stack>;
}

function CreateDialog({ open, onClose, state, busy, perform, onCreated }: { open: boolean; onClose: () => void; state: AppState; busy: boolean; perform: Perform; onCreated: (name: string) => void }) {
  const [name, setName] = useState("");
  const [device, setDevice] = useState("Nexus 5X");
  const [image, setImage] = useState("");
  useEffect(() => { if (open) { setName(nextDeviceName(state.profiles)); setDevice("Nexus 5X"); setImage(state.system_images[0]?.package_id ?? ""); } }, [open]);
  const create = async () => {
    if (!name.trim() || !image) return;
    const profileName = internalProfileName(name);
    try {
      await perform("/api/profiles", { method: "POST", body: JSON.stringify({ name: profileName, device_id: device, package_id: image }) }, () => { onCreated(profileName); onClose(); });
    } catch { /* Snackbar already shows the server message. */ }
  };
  return <Dialog open={open} onClose={busy ? undefined : onClose} maxWidth="sm" fullWidth>
    <DialogTitle>Create a new Android</DialogTitle>
    <DialogContent><Stack spacing={2.5} pt={1}>
      <TextField autoFocus label="Name" placeholder="Work phone" value={name} onChange={(event) => setName(event.target.value)} helperText="A short name you will recognize" />
      <FormControl><InputLabel>Device</InputLabel><Select label="Device" value={device} onChange={(event) => setDevice(event.target.value)}><MenuItem value="Nexus 5X">Automation phone (no cutout)</MenuItem><MenuItem value="pixel_8">Pixel 8</MenuItem><MenuItem value="pixel_7">Pixel 7</MenuItem><MenuItem value="pixel_5">Pixel 5</MenuItem><MenuItem value="pixel_tablet">Pixel Tablet</MenuItem></Select></FormControl>
      <FormControl disabled={!state.system_images.length}><InputLabel>Android version</InputLabel><Select label="Android version" value={image} onChange={(event) => setImage(event.target.value)}>{state.system_images.map((item) => <MenuItem key={item.package_id} value={item.package_id}>{item.label}</MenuItem>)}</Select></FormControl>
      {!state.system_images.length && <Alert severity="warning">No Android system image is installed. Install one with the Android SDK tools first.</Alert>}
    </Stack></DialogContent>
    <DialogActions><Button onClick={onClose} disabled={busy}>Cancel</Button><Button variant="contained" startIcon={busy ? <CircularProgress size={18} color="inherit" /> : <AddRounded />} disabled={busy || !name.trim() || !image} onClick={() => void create()}>{busy ? "Creating…" : "Create Android"}</Button></DialogActions>
  </Dialog>;
}

function CloneDialog({ source, profiles, onClose, busy, perform, onCreated }: { source: string | null; profiles: AndroidProfile[]; onClose: () => void; busy: boolean; perform: Perform; onCreated: (name: string) => void }) {
  const [name, setName] = useState("");
  useEffect(() => { if (source) setName(nextDeviceName(profiles)); }, [source]);
  const clone = async () => {
    if (!source || !name.trim()) return;
    const cloneName = internalProfileName(name);
    try {
      await perform(`/api/profiles/${encodeURIComponent(source)}/clone`, { method: "POST", body: JSON.stringify({ name: cloneName }) }, () => { onCreated(cloneName); onClose(); });
    } catch { /* Snackbar already shows the server message. */ }
  };
  return <Dialog open={Boolean(source)} onClose={busy ? undefined : onClose} maxWidth="xs" fullWidth>
    <DialogTitle>Clone {source ? displayProfileName(source) : "Android"}</DialogTitle>
    <DialogContent><Stack spacing={2} pt={1}>
      <TextField autoFocus label="Clone name" value={name} onChange={(event) => setName(event.target.value)} helperText="Apps, accounts and Android data will be copied." />
      <Alert severity="info">The source must be stopped. The clone keeps the source device type, apps and data, and receives the next available ADB slot.</Alert>
    </Stack></DialogContent>
    <DialogActions><Button onClick={onClose} disabled={busy}>Cancel</Button><Button variant="contained" startIcon={busy ? <CircularProgress size={18} color="inherit" /> : <ContentCopyRounded />} disabled={busy || !name.trim()} onClick={() => void clone()}>{busy ? "Cloning…" : "Clone Android"}</Button></DialogActions>
  </Dialog>;
}

function MonitorPage({ state }: { state: AppState }) {
  const total = state.stats.memory_total_bytes;
  const used = state.stats.memory_used_bytes;
  const running = state.profiles.filter((profile) => profile.running_serial);
  return <Stack spacing={3}>
    <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "repeat(3, 1fr)" }, gap: 2 }}>
      <Metric title="Host CPU" value={`${Math.round(state.stats.cpu_percent)}%`} progress={state.stats.cpu_percent} icon={<MemoryRounded />} />
      <Metric title="Host memory" value={`${gib(used)} / ${gib(total)} GB`} progress={total ? used / total * 100 : 0} icon={<MonitorHeartRounded />} />
      <Metric title="Running now" value={`${running.length} Android${running.length === 1 ? "" : "s"}`} progress={state.profiles.length ? running.length / state.profiles.length * 100 : 0} icon={<PhoneAndroidRounded />} />
    </Box>
    <Card><CardContent><Typography variant="h6" mb={2}>Live Androids</Typography>{running.length ? <Stack spacing={2}>{running.map((profile) => {
      const sample = state.device_stats.find((item) => item.profile_name === profile.name);
      return <Box key={profile.name}><Stack direction={{ xs: "column", md: "row" }} gap={2} alignItems={{ md: "center" }}><ListItemIcon><CheckCircleRounded color="success" /></ListItemIcon><Box sx={{ flexGrow: 1 }}><Typography fontWeight={700}>{displayProfileName(profile.name)}</Typography><Typography variant="body2" color="text.secondary">{profile.running_serial}{sample?.pid ? ` · PID ${sample.pid}` : ""}{sample?.scope_name ? ` · ${sample.scope_name}` : " · unconstrained"}</Typography></Box><Stack direction="row" gap={1} flexWrap="wrap"><Chip size="small" variant="outlined" label={`RSS ${mib(sample?.rss_bytes ?? null)}`} /><Chip size="small" variant="outlined" label={`PSS ${mib(sample?.pss_bytes ?? null)}`} /><Chip size="small" variant="outlined" label={`Swap ${mib(sample?.vm_swap_bytes ?? null)}`} /></Stack></Stack>
        {sample?.scope_name && <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr 1fr", md: "repeat(4, 1fr)" }, gap: 1, mt: 1.5 }}><Typography variant="caption">Current: {mib(sample.memory_current_bytes)}</Typography><Typography variant="caption">High: {mib(sample.memory_high_bytes)}</Typography><Typography variant="caption">Max: {mib(sample.memory_max_bytes)}</Typography><Typography variant="caption">Scope swap: {mib(sample.memory_swap_current_bytes)}</Typography><Typography variant="caption">High events: {sample.memory_events.high ?? 0}</Typography><Typography variant="caption">Max events: {sample.memory_events.max ?? 0}</Typography><Typography variant="caption">Pressure some: {(sample.pressure_some_avg10 ?? 0).toFixed(2)}%</Typography><Typography variant="caption">Pressure full: {(sample.pressure_full_avg10 ?? 0).toFixed(2)}%</Typography></Box>}
        {sample?.warning && <Alert severity="warning" sx={{ mt: 1.5 }}>{sample.warning}</Alert>}<Divider sx={{ mt: 2 }} /></Box>;
    })}</Stack> : <EmptyState icon={<PhoneAndroidRounded fontSize="large" />} title="Nothing is running" detail="Start an Android to see it here." />}</CardContent></Card>
  </Stack>;
}

function Metric({ title, value, progress, icon }: { title: string; value: string; progress: number; icon: React.ReactNode }) {
  return <Card><CardContent><Stack direction="row" gap={1} color="text.secondary" alignItems="center">{icon}<Typography variant="body2" fontWeight={700}>{title}</Typography></Stack><Typography variant="h5" my={2}>{value}</Typography><LinearProgress variant="determinate" value={Math.max(0, Math.min(100, progress))} /></CardContent></Card>;
}

function LogsPage({ state }: { state: AppState }) {
  return <Card><CardContent><Typography variant="h6">Recent activity</Typography><Typography variant="body2" color="text.secondary" mb={2}>Actions performed during this session</Typography><List disablePadding>{state.logs.length ? [...state.logs].reverse().map((entry, index) => <ListItem key={`${entry.at}-${index}`} divider={index < state.logs.length - 1}><ListItemIcon>{entry.level === "success" ? <CheckCircleRounded color="success" /> : entry.level === "error" ? <WarningAmberRounded color="error" /> : <ChevronRightRounded color="action" />}</ListItemIcon><ListItemText primary={entry.message} secondary={entry.level} /><Typography variant="caption" color="text.secondary">{relativeTime(entry.at)}</Typography></ListItem>) : <EmptyState icon={<TerminalRounded fontSize="large" />} title="No activity yet" detail="Actions will appear here." />}</List></CardContent></Card>;
}

function SettingsPage({ state, perform, busy }: { state: AppState; perform: Perform; busy: boolean }) {
  const [sdkPath, setSdkPath] = useState(state.config.android_sdk_path);
  const [jdkPath, setJdkPath] = useState(state.config.jdk_path);
  useEffect(() => { setSdkPath(state.config.android_sdk_path); }, [state.config.android_sdk_path]);
  useEffect(() => { setJdkPath(state.config.jdk_path); }, [state.config.jdk_path]);
  const dirty = sdkPath !== state.config.android_sdk_path || jdkPath !== state.config.jdk_path;
  const checks = [
    ["Android Emulator", "Runs virtual devices", state.health.emulator],
    ["ADB", "Finds and controls running devices", state.health.adb],
    ["AVD Manager", "Creates and removes Android profiles", state.health.avd_manager],
    ["KVM acceleration", "Provides fast Linux virtualization", state.health.kvm],
    ["Java", "Runs Android SDK management tools", state.health.java],
    ["systemd scopes", "Per-emulator CPU, disk, memory and swap controls", state.health.systemd_run],
  ] as const;
  return <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", lg: "minmax(0, 1.4fr) minmax(320px, 1fr)" }, gap: 3 }}>
    <Card><CardContent><Stack direction="row" alignItems="center" mb={3}><Box sx={{ flexGrow: 1 }}><Typography variant="h6">Tool locations</Typography><Typography variant="body2" color="text.secondary">EmuMi normally detects these automatically.</Typography></Box><CodeRounded color="action" /></Stack><Stack spacing={2.5}><TextField fullWidth label="Android SDK path" value={sdkPath} onChange={(event) => setSdkPath(event.target.value)} helperText="Usually /home/you/Android/Sdk" /><TextField fullWidth label="JDK path" value={jdkPath} onChange={(event) => setJdkPath(event.target.value)} helperText="Leave blank to use Java from your PATH" /><Box><Button sx={{ minWidth: 112 }} variant="contained" disabled={!dirty || busy} startIcon={busy ? <CircularProgress size={18} color="inherit" /> : undefined} onClick={() => void perform("/api/settings", { method: "POST", body: JSON.stringify({ android_sdk_path: sdkPath, jdk_path: jdkPath }) })}>{busy ? "Saving…" : dirty ? "Save paths" : "Saved"}</Button></Box></Stack></CardContent></Card>
    <Card><CardContent><Typography variant="h6">System check</Typography><Typography variant="body2" color="text.secondary" mb={2}>Everything required to create and run Androids</Typography><List disablePadding>{checks.map(([name, detail, ok]) => <ListItem key={name} disableGutters divider><ListItemIcon>{ok ? <CheckCircleRounded color="success" /> : <WarningAmberRounded color="warning" />}</ListItemIcon><ListItemText primary={name} secondary={detail} /><Chip size="small" color={ok ? "success" : "warning"} variant="outlined" label={ok ? "Ready" : "Check"} /></ListItem>)}</List><Alert severity="info" icon={<HelpOutlineRounded />} sx={{ mt: 2 }}>Changes take effect on the next refresh or launch.</Alert></CardContent></Card>
  </Box>;
}

function EmptyState({ icon, title, detail }: { icon: React.ReactNode; title: string; detail: string }) {
  return <Stack alignItems="center" justifyContent="center" textAlign="center" spacing={1} minHeight={220} color="text.secondary">{icon}<Typography variant="h6" color="text.primary">{title}</Typography><Typography variant="body2">{detail}</Typography></Stack>;
}

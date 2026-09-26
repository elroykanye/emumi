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
  CheckCircleRounded,
  ChevronRightRounded,
  CodeRounded,
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
  WarningAmberRounded,
} from "@mui/icons-material";

type Page = "androids" | "monitor" | "logs" | "settings";
type PendingAction = "starting" | "stopping";
type Speed = "Efficient" | "Balanced" | "Fast";

type AndroidProfile = {
  name: string;
  device_name: string;
  api_level: number | null;
  resolution: string | null;
  running_serial: string | null;
};

type ProfileOptions = {
  speed: Speed;
  picture: "Compact" | "Phone" | "Sharp";
  window: "Remember" | "Portrait" | "Landscape";
  cores: number;
  memory_mb: number;
  dpi: number;
  adb_port: number | null;
  gpu_mode: string;
  cold_boot: boolean;
  host_keyboard: boolean;
  device_frame: boolean;
};

type AppState = {
  profiles: AndroidProfile[];
  profile_options: Record<string, ProfileOptions>;
  system_images: { package_id: string; label: string }[];
  config: { android_sdk_path: string; jdk_path: string };
  health: { emulator: boolean; adb: boolean; avd_manager: boolean; kvm: boolean; java: boolean };
  stats: { cpu_percent: number; memory_used_bytes: number; memory_total_bytes: number };
  logs: { at: number; level: string; message: string }[];
};

const drawerWidth = 244;
const defaultOptions: ProfileOptions = {
  speed: "Balanced",
  picture: "Phone",
  window: "Remember",
  cores: 4,
  memory_mb: 4096,
  dpi: 420,
  adb_port: null,
  gpu_mode: "auto",
  cold_boot: false,
  host_keyboard: true,
  device_frame: false,
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

function statusLabel(profile: AndroidProfile, pending?: PendingAction) {
  if (pending === "starting") return "Starting…";
  if (pending === "stopping") return "Stopping…";
  return profile.running_serial ? "Running" : "Stopped";
}

function gib(bytes: number) {
  return (bytes / 1024 / 1024 / 1024).toFixed(1);
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
          if (!profile || (action === "starting" && profile.running_serial) || (action === "stopping" && !profile.running_serial)) {
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
    <Box sx={{ display: "flex", minHeight: "100vh" }}>
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

      <Box component="main" sx={{ flexGrow: 1, minWidth: 0 }}>
        <AppBar position="sticky" color="inherit" elevation={0} sx={{ borderBottom: "1px solid", borderColor: "divider" }}>
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
            <Tooltip title="Refresh now"><span><Button variant="outlined" startIcon={<RefreshRounded />} onClick={() => void refresh(true)} disabled={!state}>Refresh</Button></span></Tooltip>
            {page === "androids" && <Button variant="contained" startIcon={<AddRounded />} onClick={() => setCreateOpen(true)}>New Android</Button>}
          </Toolbar>
        </AppBar>

        <Box sx={{ p: { xs: 2, lg: 3 }, maxWidth: 1440, mx: "auto" }}>
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
            />
          ) : page === "monitor" ? (
            <MonitorPage state={state} />
          ) : page === "logs" ? (
            <LogsPage state={state} />
          ) : (
            <SettingsPage state={state} perform={perform} />
          )}
        </Box>
      </Box>

      {state && <CreateDialog open={createOpen} onClose={() => setCreateOpen(false)} state={state} busy={busy} perform={perform} onCreated={setSelectedName} />}
      <Dialog open={Boolean(deleteName)} onClose={() => setDeleteName(null)} maxWidth="xs" fullWidth>
        <DialogTitle>Delete {deleteName}?</DialogTitle>
        <DialogContent><Typography color="text.secondary">Its installed apps, files and emulator data will be permanently removed.</Typography></DialogContent>
        <DialogActions>
          <Button onClick={() => setDeleteName(null)}>Cancel</Button>
          <Button color="error" variant="contained" disabled={busy} onClick={() => deleteName && void perform(`/api/profiles/${encodeURIComponent(deleteName)}`, { method: "DELETE" }, () => setDeleteName(null))}>Delete Android</Button>
        </DialogActions>
      </Dialog>
      <Snackbar open={Boolean(notice)} autoHideDuration={3500} onClose={() => setNotice(null)} anchorOrigin={{ vertical: "bottom", horizontal: "center" }}>
        <Alert severity={notice?.error ? "error" : "success"} variant="filled" onClose={() => setNotice(null)}>{notice?.message}</Alert>
      </Snackbar>
    </Box>
  );
}

type Perform = (path: string, init: RequestInit, success?: () => void) => Promise<void>;

function AndroidsPage({ state, selected, selectedName, setSelectedName, pending, setPending, perform, setDeleteName }: {
  state: AppState;
  selected: AndroidProfile | null;
  selectedName: string | null;
  setSelectedName: (name: string) => void;
  pending: Record<string, PendingAction>;
  setPending: React.Dispatch<React.SetStateAction<Record<string, PendingAction>>>;
  perform: Perform;
  setDeleteName: (name: string) => void;
}) {
  return (
    <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", lg: "minmax(260px, 340px) minmax(0, 1fr)" }, gap: 3 }}>
      <Stack spacing={1.5}>
        <Typography variant="overline" color="text.secondary">Your devices · {state.profiles.length}</Typography>
        {state.profiles.length === 0 ? (
          <EmptyState icon={<PhoneAndroidRounded fontSize="large" />} title="No Androids yet" detail="Use New Android to create your first profile." />
        ) : state.profiles.map((profile) => {
          const running = Boolean(profile.running_serial);
          return (
            <Card key={profile.name} variant={selectedName === profile.name ? "elevation" : "outlined"} sx={{ borderColor: selectedName === profile.name ? "primary.main" : undefined }}>
              <ListItemButton selected={selectedName === profile.name} onClick={() => setSelectedName(profile.name)} sx={{ py: 1.5 }}>
                <ListItemIcon><PhoneAndroidRounded color={running ? "success" : "action"} /></ListItemIcon>
                <ListItemText primary={profile.name} secondary={`${profile.device_name || "Android device"}${profile.api_level ? ` · API ${profile.api_level}` : ""}`} primaryTypographyProps={{ fontWeight: 700 }} />
                <Chip size="small" color={running ? "success" : "default"} label={statusLabel(profile, pending[profile.name])} />
              </ListItemButton>
            </Card>
          );
        })}
      </Stack>
      {selected ? (
        <ProfileDetail profile={selected} options={state.profile_options[selected.name] ?? defaultOptions} pending={pending[selected.name]} setPending={setPending} perform={perform} setDeleteName={setDeleteName} />
      ) : (
        <EmptyState icon={<AndroidRounded fontSize="large" />} title="Choose an Android" detail="Its controls and settings will appear here." />
      )}
    </Box>
  );
}

function ProfileDetail({ profile, options, pending, setPending, perform, setDeleteName }: {
  profile: AndroidProfile;
  options: ProfileOptions;
  pending?: PendingAction;
  setPending: React.Dispatch<React.SetStateAction<Record<string, PendingAction>>>;
  perform: Perform;
  setDeleteName: (name: string) => void;
}) {
  const [draft, setDraft] = useState(options);
  useEffect(() => setDraft(options), [profile.name, options]);
  const running = Boolean(profile.running_serial);

  const toggleRun = async () => {
    const action: PendingAction = running ? "stopping" : "starting";
    setPending((value) => ({ ...value, [profile.name]: action }));
    try {
      await perform(`/api/profiles/${encodeURIComponent(profile.name)}/${running ? "stop" : "start"}`, { method: "POST" });
    } catch {
      setPending((value) => { const next = { ...value }; delete next[profile.name]; return next; });
    }
  };

  const chooseSpeed = (speed: Speed) => {
    const resources = { Efficient: [2, 2048], Balanced: [4, 4096], Fast: [8, 8192] } as const;
    setDraft((value) => ({ ...value, speed, cores: resources[speed][0], memory_mb: resources[speed][1] }));
  };

  return (
    <Stack spacing={3}>
      <Card><CardContent>
        <Stack direction={{ xs: "column", sm: "row" }} gap={2} alignItems={{ sm: "center" }}>
          <Box sx={{ width: 56, height: 56, borderRadius: 3, bgcolor: "primary.main", color: "primary.contrastText", display: "grid", placeItems: "center" }}><AndroidRounded fontSize="large" /></Box>
          <Box sx={{ flexGrow: 1 }}>
            <Typography variant="h5">{profile.name}</Typography>
            <Typography color="text.secondary">{profile.device_name || "Android device"}{profile.api_level ? ` · Android API ${profile.api_level}` : ""}</Typography>
            <Stack direction="row" gap={1} mt={1} flexWrap="wrap">
              <Chip size="small" color={running ? "success" : "default"} label={statusLabel(profile, pending)} />
              {profile.resolution && <Chip size="small" variant="outlined" label={profile.resolution} />}
              {profile.running_serial && <Chip size="small" variant="outlined" label={profile.running_serial} />}
            </Stack>
          </Box>
          <Button color="error" startIcon={<DeleteOutlineRounded />} disabled={running || Boolean(pending)} onClick={() => setDeleteName(profile.name)}>Delete</Button>
          <Button variant="contained" color={running ? "error" : "primary"} startIcon={pending ? <CircularProgress size={18} color="inherit" /> : running ? <StopRounded /> : <PlayArrowRounded />} disabled={Boolean(pending)} onClick={() => void toggleRun()}>{statusLabel(profile, pending) === "Stopped" ? "Start" : statusLabel(profile, pending) === "Running" ? "Stop" : statusLabel(profile, pending)}</Button>
        </Stack>
      </CardContent></Card>

      <Card><CardContent>
        <Stack direction="row" alignItems="center" mb={2}>
          <Box sx={{ flexGrow: 1 }}><Typography variant="h6">Quick setup</Typography><Typography variant="body2" color="text.secondary">The useful controls, without emulator jargon.</Typography></Box>
          <Button variant="contained" onClick={() => void perform(`/api/profiles/${encodeURIComponent(profile.name)}/settings`, { method: "POST", body: JSON.stringify(draft) })}>Save changes</Button>
        </Stack>
        <Divider />
        <SettingRow icon={<MemoryRounded />} title="Performance" description="Choose how much of this computer Android may use">
          <ToggleButtonGroup exclusive value={draft.speed} size="small" onChange={(_, value: Speed | null) => value && chooseSpeed(value)}>
            <ToggleButton value="Efficient">Efficient</ToggleButton><ToggleButton value="Balanced">Balanced</ToggleButton><ToggleButton value="Fast">Fast</ToggleButton>
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
        <Accordion disableGutters elevation={0} sx={{ mt: 1, "&:before": { display: "none" } }}>
          <AccordionSummary expandIcon={<ExpandMoreRounded />}><TuneRounded sx={{ mr: 2 }} /><Box><Typography fontWeight={700}>Advanced settings</Typography><Typography variant="body2" color="text.secondary">CPU, memory, graphics, ports and boot behavior</Typography></Box></AccordionSummary>
          <AccordionDetails>
            <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", sm: "1fr 1fr" }, gap: 2 }}>
              <TextField label="CPU cores" type="number" value={draft.cores} onChange={(event) => setDraft({ ...draft, cores: Number(event.target.value) })} slotProps={{ htmlInput: { min: 1, max: 16 } }} />
              <TextField label="Memory (MB)" type="number" value={draft.memory_mb} onChange={(event) => setDraft({ ...draft, memory_mb: Number(event.target.value) })} slotProps={{ htmlInput: { min: 512, max: 16384, step: 256 } }} />
              <FormControl><InputLabel>Graphics</InputLabel><Select label="Graphics" value={draft.gpu_mode} onChange={(event) => setDraft({ ...draft, gpu_mode: event.target.value })}><MenuItem value="auto">Automatic</MenuItem><MenuItem value="host">Hardware</MenuItem><MenuItem value="swiftshader_indirect">Software</MenuItem></Select></FormControl>
              <TextField label="ADB port" type="number" value={draft.adb_port ?? ""} placeholder="Automatic" onChange={(event) => setDraft({ ...draft, adb_port: event.target.value ? Number(event.target.value) : null })} />
              <FormControlLabel control={<Switch checked={draft.cold_boot} onChange={(event) => setDraft({ ...draft, cold_boot: event.target.checked })} />} label="Cold boot next time" />
            </Box>
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
  const [device, setDevice] = useState("pixel_8");
  const [image, setImage] = useState("");
  useEffect(() => { if (open) { setName(""); setDevice("pixel_8"); setImage(state.system_images[0]?.package_id ?? ""); } }, [open, state.system_images]);
  const create = async () => {
    if (!name.trim() || !image) return;
    try {
      await perform("/api/profiles", { method: "POST", body: JSON.stringify({ name: name.trim(), device_id: device, package_id: image }) }, () => { onCreated(name.trim()); onClose(); });
    } catch { /* Snackbar already shows the server message. */ }
  };
  return <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
    <DialogTitle>Create a new Android</DialogTitle>
    <DialogContent><Stack spacing={2.5} pt={1}>
      <TextField autoFocus label="Name" placeholder="Work phone" value={name} onChange={(event) => setName(event.target.value)} helperText="A short name you will recognize" />
      <FormControl><InputLabel>Device</InputLabel><Select label="Device" value={device} onChange={(event) => setDevice(event.target.value)}><MenuItem value="pixel_8">Pixel 8</MenuItem><MenuItem value="pixel_7">Pixel 7</MenuItem><MenuItem value="pixel_5">Pixel 5</MenuItem><MenuItem value="pixel_tablet">Pixel Tablet</MenuItem></Select></FormControl>
      <FormControl disabled={!state.system_images.length}><InputLabel>Android version</InputLabel><Select label="Android version" value={image} onChange={(event) => setImage(event.target.value)}>{state.system_images.map((item) => <MenuItem key={item.package_id} value={item.package_id}>{item.label}</MenuItem>)}</Select></FormControl>
      {!state.system_images.length && <Alert severity="warning">No Android system image is installed. Install one with the Android SDK tools first.</Alert>}
    </Stack></DialogContent>
    <DialogActions><Button onClick={onClose}>Cancel</Button><Button variant="contained" startIcon={busy ? <CircularProgress size={18} color="inherit" /> : <AddRounded />} disabled={busy || !name.trim() || !image} onClick={() => void create()}>Create Android</Button></DialogActions>
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
    <Card><CardContent><Typography variant="h6" mb={2}>Live Androids</Typography>{running.length ? <List disablePadding>{running.map((profile) => <ListItem key={profile.name} divider><ListItemIcon><CheckCircleRounded color="success" /></ListItemIcon><ListItemText primary={profile.name} secondary={profile.device_name} /><Chip size="small" color="success" label={profile.running_serial} /></ListItem>)}</List> : <EmptyState icon={<PhoneAndroidRounded fontSize="large" />} title="Nothing is running" detail="Start an Android to see it here." />}</CardContent></Card>
  </Stack>;
}

function Metric({ title, value, progress, icon }: { title: string; value: string; progress: number; icon: React.ReactNode }) {
  return <Card><CardContent><Stack direction="row" gap={1} color="text.secondary" alignItems="center">{icon}<Typography variant="body2" fontWeight={700}>{title}</Typography></Stack><Typography variant="h5" my={2}>{value}</Typography><LinearProgress variant="determinate" value={Math.max(0, Math.min(100, progress))} /></CardContent></Card>;
}

function LogsPage({ state }: { state: AppState }) {
  return <Card><CardContent><Typography variant="h6">Recent activity</Typography><Typography variant="body2" color="text.secondary" mb={2}>Actions performed during this session</Typography><List disablePadding>{state.logs.length ? [...state.logs].reverse().map((entry, index) => <ListItem key={`${entry.at}-${index}`} divider={index < state.logs.length - 1}><ListItemIcon>{entry.level === "success" ? <CheckCircleRounded color="success" /> : entry.level === "error" ? <WarningAmberRounded color="error" /> : <ChevronRightRounded color="action" />}</ListItemIcon><ListItemText primary={entry.message} secondary={entry.level} /><Typography variant="caption" color="text.secondary">{relativeTime(entry.at)}</Typography></ListItem>) : <EmptyState icon={<TerminalRounded fontSize="large" />} title="No activity yet" detail="Actions will appear here." />}</List></CardContent></Card>;
}

function SettingsPage({ state, perform }: { state: AppState; perform: Perform }) {
  const [sdkPath, setSdkPath] = useState(state.config.android_sdk_path);
  const [jdkPath, setJdkPath] = useState(state.config.jdk_path);
  useEffect(() => { setSdkPath(state.config.android_sdk_path); setJdkPath(state.config.jdk_path); }, [state.config]);
  const checks = [
    ["Android Emulator", "Runs virtual devices", state.health.emulator],
    ["ADB", "Finds and controls running devices", state.health.adb],
    ["AVD Manager", "Creates and removes Android profiles", state.health.avd_manager],
    ["KVM acceleration", "Provides fast Linux virtualization", state.health.kvm],
    ["Java", "Runs Android SDK management tools", state.health.java],
  ] as const;
  return <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", lg: "minmax(0, 1.4fr) minmax(320px, 1fr)" }, gap: 3 }}>
    <Card><CardContent><Stack direction="row" alignItems="center" mb={3}><Box sx={{ flexGrow: 1 }}><Typography variant="h6">Tool locations</Typography><Typography variant="body2" color="text.secondary">EmuMi normally detects these automatically.</Typography></Box><CodeRounded color="action" /></Stack><Stack spacing={2.5}><TextField fullWidth label="Android SDK path" value={sdkPath} onChange={(event) => setSdkPath(event.target.value)} helperText="Usually /home/you/Android/Sdk" /><TextField fullWidth label="JDK path" value={jdkPath} onChange={(event) => setJdkPath(event.target.value)} helperText="Leave blank to use Java from your PATH" /><Box><Button variant="contained" onClick={() => void perform("/api/settings", { method: "POST", body: JSON.stringify({ android_sdk_path: sdkPath, jdk_path: jdkPath }) })}>Save paths</Button></Box></Stack></CardContent></Card>
    <Card><CardContent><Typography variant="h6">System check</Typography><Typography variant="body2" color="text.secondary" mb={2}>Everything required to create and run Androids</Typography><List disablePadding>{checks.map(([name, detail, ok]) => <ListItem key={name} disableGutters divider><ListItemIcon>{ok ? <CheckCircleRounded color="success" /> : <WarningAmberRounded color="warning" />}</ListItemIcon><ListItemText primary={name} secondary={detail} /><Chip size="small" color={ok ? "success" : "warning"} variant="outlined" label={ok ? "Ready" : "Check"} /></ListItem>)}</List><Alert severity="info" icon={<HelpOutlineRounded />} sx={{ mt: 2 }}>Changes take effect on the next refresh or launch.</Alert></CardContent></Card>
  </Box>;
}

function EmptyState({ icon, title, detail }: { icon: React.ReactNode; title: string; detail: string }) {
  return <Stack alignItems="center" justifyContent="center" textAlign="center" spacing={1} minHeight={220} color="text.secondary">{icon}<Typography variant="h6" color="text.primary">{title}</Typography><Typography variant="body2">{detail}</Typography></Stack>;
}

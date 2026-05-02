use std::env;
use std::ffi::{c_char, c_int, c_short, c_void, CString};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[repr(C)]
struct DozedWl {
	_private: [u8; 0],
}

#[repr(C)]
struct DozedNotification {
	_private: [u8; 0],
}

#[repr(C)]
struct DozedLogind {
	_private: [u8; 0],
}

#[repr(C)]
struct PollFd {
	fd: c_int,
	events: c_short,
	revents: c_short,
}

#[repr(C)]
struct Tm {
	tm_sec: c_int,
	tm_min: c_int,
	tm_hour: c_int,
	tm_mday: c_int,
	tm_mon: c_int,
	tm_year: c_int,
	tm_wday: c_int,
	tm_yday: c_int,
	tm_isdst: c_int,
	tm_gmtoff: isize,
	tm_zone: *const c_char,
}

type IdleCallback = extern "C" fn(*mut c_void, c_int);
#[cfg(have_logind)]
type LogindCallback = extern "C" fn(*mut c_void, c_int);

unsafe extern "C" {
	fn dozed_wayland_connect(seat_name: *const c_char) -> *mut DozedWl;
	fn dozed_wayland_destroy(ctx: *mut DozedWl);
	fn dozed_wayland_fd(ctx: *mut DozedWl) -> c_int;
	fn dozed_wayland_has_idle(ctx: *mut DozedWl) -> c_int;
	fn dozed_wayland_has_seat(ctx: *mut DozedWl) -> c_int;
	fn dozed_wayland_has_toplevel_manager(ctx: *mut DozedWl) -> c_int;
	fn dozed_wayland_has_fullscreen(ctx: *mut DozedWl) -> c_int;
	fn dozed_wayland_flush(ctx: *mut DozedWl) -> c_int;
	fn dozed_wayland_dispatch(ctx: *mut DozedWl) -> c_int;
	fn dozed_wayland_roundtrip(ctx: *mut DozedWl) -> c_int;
	fn dozed_notification_create(
		ctx: *mut DozedWl,
		timeout_ms: c_int,
		obey_inhibitors: c_int,
		callback: IdleCallback,
		data: *mut c_void,
	) -> *mut DozedNotification;
	fn dozed_notification_destroy(notif: *mut DozedNotification);

	#[cfg(have_logind)]
	fn dozed_logind_connect(
		want_sleep: c_int,
		want_lock: c_int,
		want_unlock: c_int,
		callback: LogindCallback,
		data: *mut c_void,
	) -> *mut DozedLogind;
	fn dozed_logind_destroy(ctx: *mut DozedLogind);
	fn dozed_logind_fd(ctx: *mut DozedLogind) -> c_int;
	fn dozed_logind_process(ctx: *mut DozedLogind) -> c_int;

	fn poll(fds: *mut PollFd, nfds: usize, timeout: c_int) -> c_int;
	fn signal(signum: c_int, handler: extern "C" fn(c_int)) -> usize;
	fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
	fn strftime(s: *mut c_char, max: usize, format: *const c_char, tm: *const Tm) -> usize;
}

const POLLIN: c_short = 0x001;
const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;
const SIGUSR1: c_int = 10;
const EINTR: i32 = 4;

static TERMINATE: AtomicBool = AtomicBool::new(false);
static FORCE_IDLE: AtomicBool = AtomicBool::new(false);
static mut APP: *mut App = ptr::null_mut();

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LogLevel {
	Error,
	Info,
	Debug,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FullscreenPolicy {
	Suppress,
	Ignore,
}

struct TimeoutCmd {
	timeout_ms: i32,
	registered_timeout_ms: i32,
	idle_cmd: Option<String>,
	resume_cmd: Option<String>,
	resume_pending: bool,
	suppressed_by_fullscreen: bool,
	notification: *mut DozedNotification,
}

#[derive(Default)]
struct Config {
	wait: bool,
	debug: bool,
	no_config: bool,
	dry_run: bool,
	print_events: bool,
	validate_config: bool,
	fullscreen_policy: FullscreenPolicy,
	seat_name: Option<String>,
	config_path: Option<String>,
	timeouts: Vec<TimeoutCmd>,
	before_sleep_cmd: Option<String>,
	after_resume_cmd: Option<String>,
	lock_cmd: Option<String>,
	unlock_cmd: Option<String>,
}

struct App {
	wl: *mut DozedWl,
	logind: *mut DozedLogind,
	timeouts: Vec<TimeoutCmd>,
	wait: bool,
	before_sleep_cmd: Option<String>,
	after_resume_cmd: Option<String>,
	lock_cmd: Option<String>,
	unlock_cmd: Option<String>,
	log_level: LogLevel,
	fullscreen_active: bool,
	fullscreen_policy: FullscreenPolicy,
}

impl Default for FullscreenPolicy {
	fn default() -> Self {
		Self::Suppress
	}
}

impl Drop for App {
	fn drop(&mut self) {
		for cmd in &mut self.timeouts {
			destroy_notification(cmd);
		}
		unsafe {
			if !self.logind.is_null() {
				dozed_logind_destroy(self.logind);
			}
			if !self.wl.is_null() {
				dozed_wayland_destroy(self.wl);
			}
		}
	}
}

impl App {
	fn new(config: Config, wl: *mut DozedWl) -> Self {
		let log_level = if config.debug { LogLevel::Debug } else { LogLevel::Info };
		Self {
			wl,
			logind: ptr::null_mut(),
			timeouts: config.timeouts,
			wait: config.wait,
			before_sleep_cmd: config.before_sleep_cmd,
			after_resume_cmd: config.after_resume_cmd,
			lock_cmd: config.lock_cmd,
			unlock_cmd: config.unlock_cmd,
			log_level,
			fullscreen_active: false,
			fullscreen_policy: config.fullscreen_policy,
		}
	}

	fn setup_logind(&mut self) -> Result<(), String> {
		let need_sleep = self.before_sleep_cmd.is_some() || self.after_resume_cmd.is_some();
		let need_lock = self.lock_cmd.is_some();
		let need_unlock = self.unlock_cmd.is_some();
		if !need_sleep && !need_lock && !need_unlock {
			return Ok(());
		}

		#[cfg(not(have_logind))]
		{
			Err("logind support was not built in".to_string())
		}

		#[cfg(have_logind)]
		unsafe {
			self.logind = dozed_logind_connect(
				need_sleep as c_int,
				need_lock as c_int,
				need_unlock as c_int,
				logind_callback,
				ptr::null_mut(),
			);
			if self.logind.is_null() {
				return Err("failed to connect to login1 D-Bus".to_string());
			}
			Ok(())
		}
	}

	fn register_all(&mut self) {
		for idx in 0..self.timeouts.len() {
			self.register_timeout(idx, self.timeouts[idx].timeout_ms, true);
		}
	}

	fn register_timeout(&mut self, idx: usize, timeout_ms: i32, obey_inhibitors: bool) {
		if idx >= self.timeouts.len() {
			return;
		}
		destroy_notification(&mut self.timeouts[idx]);
		if timeout_ms < 0 {
			return;
		}
		log(self.log_level, LogLevel::Debug, &format!("register timeout: {timeout_ms} ms"));
		let data = (idx + 1) as *mut c_void;
		let notification = unsafe {
			dozed_notification_create(
				self.wl,
				timeout_ms,
				obey_inhibitors as c_int,
				idle_callback,
				data,
			)
		};
		self.timeouts[idx].notification = notification;
		self.timeouts[idx].registered_timeout_ms = timeout_ms;
	}

	fn handle_idle(&mut self, idx: usize) {
		if idx >= self.timeouts.len() {
			return;
		}
		if self.fullscreen_policy == FullscreenPolicy::Suppress
			&& unsafe { dozed_wayland_has_fullscreen(self.wl) } != 0
		{
			self.fullscreen_active = true;
			log(
				self.log_level,
				LogLevel::Debug,
				"fullscreen toplevel active; skipping idle command",
			);
			self.timeouts[idx].suppressed_by_fullscreen = true;
			destroy_notification(&mut self.timeouts[idx]);
			return;
		}
		self.timeouts[idx].resume_pending = true;
		self.timeouts[idx].suppressed_by_fullscreen = false;
		log(self.log_level, LogLevel::Debug, "idle state");
		if let Some(cmd) = self.timeouts[idx].idle_cmd.clone() {
			exec_command(&cmd, self.wait, self.log_level);
		}
	}

	fn handle_resume(&mut self, idx: usize) {
		if idx >= self.timeouts.len() {
			return;
		}
		self.timeouts[idx].resume_pending = false;
		self.timeouts[idx].suppressed_by_fullscreen = false;
		log(self.log_level, LogLevel::Debug, "active state");
		if self.timeouts[idx].registered_timeout_ms != self.timeouts[idx].timeout_ms {
			self.register_timeout(idx, self.timeouts[idx].timeout_ms, true);
		}
		if let Some(cmd) = self.timeouts[idx].resume_cmd.clone() {
			exec_command(&cmd, self.wait, self.log_level);
		}
	}

	fn run_pending_resume(&mut self) {
		for idx in 0..self.timeouts.len() {
			if self.timeouts[idx].resume_pending {
				self.handle_resume(idx);
			}
		}
	}

	fn force_idle(&mut self) {
		for idx in 0..self.timeouts.len() {
			self.register_timeout(idx, 0, false);
		}
	}

	fn sync_fullscreen_state(&mut self) {
		if self.fullscreen_policy == FullscreenPolicy::Ignore {
			return;
		}
		let active = unsafe { dozed_wayland_has_fullscreen(self.wl) } != 0;
		if self.fullscreen_active && !active {
			for idx in 0..self.timeouts.len() {
				if self.timeouts[idx].suppressed_by_fullscreen {
					self.timeouts[idx].suppressed_by_fullscreen = false;
					self.register_timeout(idx, self.timeouts[idx].timeout_ms, true);
				}
			}
			unsafe {
				dozed_wayland_flush(self.wl);
			}
		}
		self.fullscreen_active = active;
	}

	#[cfg(have_logind)]
	fn handle_logind_event(&mut self, event: c_int) {
		match event {
			1 => {
				if let Some(cmd) = self.before_sleep_cmd.clone() {
					exec_command(&cmd, self.wait, self.log_level);
				}
			}
			2 => {
				if let Some(cmd) = self.after_resume_cmd.clone() {
					exec_command(&cmd, self.wait, self.log_level);
				}
			}
			3 => {
				if let Some(cmd) = self.lock_cmd.clone() {
					exec_command(&cmd, self.wait, self.log_level);
				}
			}
			4 => {
				if let Some(cmd) = self.unlock_cmd.clone() {
					exec_command(&cmd, self.wait, self.log_level);
				}
			}
			_ => {}
		}
	}

	fn event_loop(&mut self) -> Result<(), String> {
		self.register_all();
		unsafe {
			dozed_wayland_roundtrip(self.wl);
			dozed_wayland_flush(self.wl);
		}

		while !TERMINATE.load(Ordering::SeqCst) {
			if FORCE_IDLE.swap(false, Ordering::SeqCst) {
				self.force_idle();
				unsafe {
					dozed_wayland_flush(self.wl);
				}
				self.sync_fullscreen_state();
			}

			let mut fds = vec![PollFd {
				fd: unsafe { dozed_wayland_fd(self.wl) },
				events: POLLIN,
				revents: 0,
			}];

			if !self.logind.is_null() {
				let fd = unsafe { dozed_logind_fd(self.logind) };
				if fd >= 0 {
					fds.push(PollFd {
						fd,
						events: POLLIN,
						revents: 0,
					});
				}
			}

			let ret = unsafe { poll(fds.as_mut_ptr(), fds.len(), -1) };
			if ret < 0 {
				let err = io::Error::last_os_error();
				if err.raw_os_error() == Some(EINTR) {
					continue;
				}
				return Err(format!("poll failed: {err}"));
			}

			if fds[0].revents & POLLIN != 0 {
				let ret = unsafe { dozed_wayland_dispatch(self.wl) };
				if ret < 0 {
					return Err("Wayland dispatch failed".to_string());
				}
				unsafe {
					dozed_wayland_flush(self.wl);
				}
				self.sync_fullscreen_state();
			}

			if fds.len() > 1 && fds[1].revents & POLLIN != 0 {
				let ret = unsafe { dozed_logind_process(self.logind) };
				if ret < 0 {
					return Err("login1 D-Bus dispatch failed".to_string());
				}
			}
		}

		self.run_pending_resume();
		Ok(())
	}
}

impl Default for TimeoutCmd {
	fn default() -> Self {
		Self {
			timeout_ms: -1,
			registered_timeout_ms: -1,
			idle_cmd: None,
			resume_cmd: None,
			resume_pending: false,
			suppressed_by_fullscreen: false,
			notification: ptr::null_mut(),
		}
	}
}

extern "C" fn idle_callback(data: *mut c_void, idled: c_int) {
	let idx = data as usize;
	if idx == 0 {
		return;
	}
	unsafe {
		if APP.is_null() {
			return;
		}
		if idled != 0 {
			(*APP).handle_idle(idx - 1);
		} else {
			(*APP).handle_resume(idx - 1);
		}
	}
}

#[cfg(have_logind)]
extern "C" fn logind_callback(_data: *mut c_void, event: c_int) {
	unsafe {
		if !APP.is_null() {
			(*APP).handle_logind_event(event);
		}
	}
}

extern "C" fn signal_handler(sig: c_int) {
	match sig {
		SIGINT | SIGTERM => TERMINATE.store(true, Ordering::SeqCst),
		SIGUSR1 => FORCE_IDLE.store(true, Ordering::SeqCst),
		_ => {}
	}
}

fn destroy_notification(cmd: &mut TimeoutCmd) {
	if !cmd.notification.is_null() {
		unsafe {
			dozed_notification_destroy(cmd.notification);
		}
		cmd.notification = ptr::null_mut();
	}
}

fn exec_command(command: &str, wait: bool, level: LogLevel) {
	log(level, LogLevel::Debug, &format!("exec: {command}"));
	let mut child = match Command::new("sh").arg("-c").arg(command).spawn() {
		Ok(child) => child,
		Err(err) => {
			log(level, LogLevel::Error, &format!("failed to execute command: {err}"));
			return;
		}
	};

	if wait {
		match child.wait() {
			Ok(status) => log(level, LogLevel::Debug, &format!("process exited: {status}")),
			Err(err) => log(level, LogLevel::Error, &format!("failed to wait for command: {err}")),
		}
	} else {
		thread::spawn(move || {
			let _ = child.wait();
		});
	}
}

fn log(current: LogLevel, level: LogLevel, message: &str) {
	if level > current {
		return;
	}
	eprintln!("{} - {message}", local_timestamp());
}

fn local_timestamp() -> String {
	let secs = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs() as i64)
		.unwrap_or(0);
	let format = b"%F %T\0";
	let mut tm = Tm {
		tm_sec: 0,
		tm_min: 0,
		tm_hour: 0,
		tm_mday: 0,
		tm_mon: 0,
		tm_year: 0,
		tm_wday: 0,
		tm_yday: 0,
		tm_isdst: 0,
		tm_gmtoff: 0,
		tm_zone: ptr::null(),
	};
	let mut buf = [0u8; 32];
	let len = unsafe {
		if localtime_r(&secs, &mut tm).is_null() {
			0
		} else {
			strftime(
				buf.as_mut_ptr() as *mut c_char,
				buf.len(),
				format.as_ptr() as *const c_char,
				&tm,
			)
		}
	};
	if len == 0 {
		secs.to_string()
	} else {
		String::from_utf8_lossy(&buf[..len]).into_owned()
	}
}

fn parse_args(argv: &[String]) -> Result<Config, String> {
	let mut config = Config::default();
	let mut events = Vec::new();
	let mut i = 1;
	while i < argv.len() {
		match argv[i].as_str() {
			"-h" | "--help" => {
				print_help(&argv[0]);
				std::process::exit(0);
			}
			"-d" => {
				config.debug = true;
				i += 1;
			}
			"-w" => {
				config.wait = true;
				i += 1;
			}
			"--no-config" => {
				config.no_config = true;
				i += 1;
			}
			"--dry-run" => {
				config.dry_run = true;
				config.print_events = true;
				config.validate_config = true;
				i += 1;
			}
			"--print-events" => {
				config.print_events = true;
				i += 1;
			}
			"--validate-config" => {
				config.validate_config = true;
				i += 1;
			}
			"--ignore-fullscreen" => {
				config.fullscreen_policy = FullscreenPolicy::Ignore;
				i += 1;
			}
			"--fullscreen-policy" => {
				i += 1;
				if i >= argv.len() {
					return Err("--fullscreen-policy requires suppress or ignore".to_string());
				}
				config.fullscreen_policy = parse_fullscreen_policy(&argv[i])?;
				i += 1;
			}
			arg if arg.starts_with("--fullscreen-policy=") => {
				let value = arg
					.split_once('=')
					.map(|(_, value)| value)
					.unwrap_or_default();
				config.fullscreen_policy = parse_fullscreen_policy(value)?;
				i += 1;
			}
			"-C" => {
				i += 1;
				if i >= argv.len() {
					return Err("-C requires a path".to_string());
				}
				config.config_path = Some(argv[i].clone());
				i += 1;
			}
			"-S" => {
				i += 1;
				if i >= argv.len() {
					return Err("-S requires a seat name".to_string());
				}
				config.seat_name = Some(argv[i].clone());
				i += 1;
			}
			arg if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
			_ => {
				events.extend_from_slice(&argv[i..]);
				break;
			}
		}
	}

	parse_events(&mut config, &events)?;
	Ok(config)
}

fn parse_fullscreen_policy(value: &str) -> Result<FullscreenPolicy, String> {
	match value {
		"suppress" => Ok(FullscreenPolicy::Suppress),
		"ignore" => Ok(FullscreenPolicy::Ignore),
		_ => Err(format!(
			"invalid fullscreen policy '{value}', expected suppress or ignore"
		)),
	}
}

fn parse_events(config: &mut Config, args: &[String]) -> Result<(), String> {
	let mut i = 0;
	while i < args.len() {
		match args[i].as_str() {
			"timeout" => {
				if i + 2 >= args.len() {
					return Err("timeout requires <seconds> <command>".to_string());
				}
				let seconds = parse_seconds(&args[i + 1], "timeout")?;
				let mut cmd = TimeoutCmd {
					timeout_ms: if seconds > 0 { seconds * 1000 } else { -1 },
					idle_cmd: Some(args[i + 2].clone()),
					..Default::default()
				};
				i += 3;
				if i + 1 < args.len() && args[i] == "resume" {
					cmd.resume_cmd = Some(args[i + 1].clone());
					i += 2;
				}
				config.timeouts.push(cmd);
			}
			"before-sleep" => {
				if i + 1 >= args.len() {
					return Err("before-sleep requires <command>".to_string());
				}
				config.before_sleep_cmd = Some(args[i + 1].clone());
				i += 2;
			}
			"after-resume" => {
				if i + 1 >= args.len() {
					return Err("after-resume requires <command>".to_string());
				}
				config.after_resume_cmd = Some(args[i + 1].clone());
				i += 2;
			}
			"lock" => {
				if i + 1 >= args.len() {
					return Err("lock requires <command>".to_string());
				}
				config.lock_cmd = Some(args[i + 1].clone());
				i += 2;
			}
			"unlock" => {
				if i + 1 >= args.len() {
					return Err("unlock requires <command>".to_string());
				}
				config.unlock_cmd = Some(args[i + 1].clone());
				i += 2;
			}
			other => return Err(format!("unsupported command: {other}")),
		}
	}
	Ok(())
}

fn parse_seconds(value: &str, name: &str) -> Result<i32, String> {
	let seconds: i32 = value
		.parse()
		.map_err(|_| format!("invalid {name} value '{value}', expected seconds"))?;
	if seconds < 0 {
		return Err(format!("invalid {name} value '{value}', expected seconds"));
	}
	Ok(seconds)
}

fn load_config(config: &mut Config) -> Result<(), String> {
	if config.no_config {
		return Ok(());
	}
	let path = match config.config_path.clone().or_else(default_config_path) {
		Some(path) => path,
		None => return Ok(()),
	};
	let content = fs::read_to_string(&path)
		.map_err(|err| format!("failed to read config {path}: {err}"))?;

	for (lineno, line) in content.lines().enumerate() {
		let trimmed = line.trim();
		if trimmed.is_empty() || trimmed.starts_with('#') {
			continue;
		}
		let words = shell_words(trimmed)
			.map_err(|err| format!("{path}:{}: {err}", lineno + 1))?;
		parse_events(config, &words)
			.map_err(|err| format!("{path}:{}: {err}", lineno + 1))?;
	}
	Ok(())
}

fn has_events(config: &Config) -> bool {
	!config.timeouts.is_empty()
		|| config.before_sleep_cmd.is_some()
		|| config.after_resume_cmd.is_some()
		|| config.lock_cmd.is_some()
		|| config.unlock_cmd.is_some()
}

fn print_events(config: &Config) {
	println!("fullscreen-policy {}", fullscreen_policy_name(config.fullscreen_policy));
	for timeout in &config.timeouts {
		let seconds = if timeout.timeout_ms < 0 {
			0
		} else {
			timeout.timeout_ms / 1000
		};
		match (&timeout.idle_cmd, &timeout.resume_cmd) {
			(Some(idle), Some(resume)) => {
				println!("timeout {seconds} {idle:?} resume {resume:?}");
			}
			(Some(idle), None) => println!("timeout {seconds} {idle:?}"),
			_ => {}
		}
	}
	if let Some(cmd) = &config.before_sleep_cmd {
		println!("before-sleep {cmd:?}");
	}
	if let Some(cmd) = &config.after_resume_cmd {
		println!("after-resume {cmd:?}");
	}
	if let Some(cmd) = &config.lock_cmd {
		println!("lock {cmd:?}");
	}
	if let Some(cmd) = &config.unlock_cmd {
		println!("unlock {cmd:?}");
	}
}

fn fullscreen_policy_name(policy: FullscreenPolicy) -> &'static str {
	match policy {
		FullscreenPolicy::Suppress => "suppress",
		FullscreenPolicy::Ignore => "ignore",
	}
}

fn default_config_path() -> Option<String> {
	let mut paths = Vec::new();
	if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
		if !xdg.is_empty() {
			paths.push(PathBuf::from(xdg).join("dozed/config"));
		}
	}
	if let Ok(home) = env::var("HOME") {
		paths.push(PathBuf::from(&home).join(".config/dozed/config"));
		paths.push(PathBuf::from(home).join(".dozed/config"));
	}
	paths.push(PathBuf::from("/etc/dozed/config"));

	paths
		.into_iter()
		.find(|path| path.is_file())
		.map(|path| path.to_string_lossy().into_owned())
}

fn shell_words(line: &str) -> Result<Vec<String>, String> {
	let mut words = Vec::new();
	let mut current = String::new();
	let mut chars = line.chars().peekable();
	let mut quote = None;

	while let Some(ch) = chars.next() {
		match (quote, ch) {
			(None, '#') if current.is_empty() => break,
			(None, c) if c.is_whitespace() => {
				if !current.is_empty() {
					words.push(std::mem::take(&mut current));
				}
			}
			(None, '\'' | '"') => quote = Some(ch),
			(Some(q), c) if c == q => quote = None,
			(_, '\\') => {
				if let Some(next) = chars.next() {
					current.push(next);
				}
			}
			(_, c) => current.push(c),
		}
	}

	if let Some(q) = quote {
		return Err(format!("unterminated {q} quote"));
	}
	if !current.is_empty() {
		words.push(current);
	}
	Ok(words)
}

fn print_help(name: &str) {
	println!("Usage: {name} [OPTIONS] [EVENTS...]");
	println!("  -h, --help                         this help menu");
	println!("  -C <path>                          path to config file");
	println!("  -d                                 debug output");
	println!("  -w                                 wait for command completion");
	println!("  -S <seat>                          pick the Wayland seat to watch");
	println!("      --no-config                    ignore config files");
	println!("      --dry-run                      validate and print events, then exit");
	println!("      --validate-config              validate config and arguments, then exit");
	println!("      --print-events                 print parsed events, then exit");
	println!("      --ignore-fullscreen            do not suppress idle in fullscreen apps");
	println!("      --fullscreen-policy <policy>   suppress or ignore");
}

fn main() {
	unsafe {
		signal(SIGINT, signal_handler);
		signal(SIGTERM, signal_handler);
		signal(SIGUSR1, signal_handler);
	}

	let argv = env::args().collect::<Vec<_>>();
	let mut config = match parse_args(&argv) {
		Ok(config) => config,
		Err(err) => {
			eprintln!("{err}");
			std::process::exit(1);
		}
	};
	if let Err(err) = load_config(&mut config) {
		eprintln!("{err}");
		std::process::exit(1);
	}
	if config.print_events {
		print_events(&config);
	}
	if config.validate_config || config.print_events || config.dry_run {
		return;
	}

	let seat_name = match config.seat_name.as_deref() {
		Some(name) => match CString::new(name) {
			Ok(name) => Some(name),
			Err(_) => {
				eprintln!("seat name contains an interior NUL byte");
				std::process::exit(1);
			}
		},
		None => None,
	};
	let wl = unsafe {
		dozed_wayland_connect(
			seat_name
				.as_ref()
				.map(|name| name.as_ptr())
				.unwrap_or(ptr::null()),
		)
	};
	if wl.is_null() {
		eprintln!("unable to connect to the Wayland compositor");
		std::process::exit(3);
	}
	if unsafe { dozed_wayland_has_idle(wl) } == 0 {
		eprintln!("compositor does not support ext-idle-notify-v1");
		unsafe {
			dozed_wayland_destroy(wl);
		}
		std::process::exit(4);
	}
	if unsafe { dozed_wayland_has_seat(wl) } == 0 {
		eprintln!("requested Wayland seat was not found");
		unsafe {
			dozed_wayland_destroy(wl);
		}
		std::process::exit(5);
	}
	if config.debug
		&& config.fullscreen_policy == FullscreenPolicy::Suppress
		&& unsafe { dozed_wayland_has_toplevel_manager(wl) } == 0
	{
		eprintln!(
			"fullscreen suppression unavailable: compositor does not expose zwlr_foreign_toplevel_manager_v1"
		);
	}

	if !has_events(&config) {
		eprintln!("no command specified; nothing to do");
		unsafe {
			dozed_wayland_destroy(wl);
		}
		return;
	}

	let mut app = App::new(config, wl);
	unsafe {
		APP = &mut app;
	}

	if let Err(err) = app.setup_logind().and_then(|_| app.event_loop()) {
		eprintln!("{err}");
		unsafe {
			APP = ptr::null_mut();
		}
		std::process::exit(1);
	}

	unsafe {
		APP = ptr::null_mut();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn args(values: &[&str]) -> Vec<String> {
		values.iter().map(|value| value.to_string()).collect()
	}

	#[test]
	fn parses_timeout_with_resume() {
		let config = parse_args(&args(&[
			"dozed",
			"--no-config",
			"timeout",
			"5",
			"lock",
			"resume",
			"wake",
		]))
		.unwrap();

		assert!(config.no_config);
		assert_eq!(config.timeouts.len(), 1);
		assert_eq!(config.timeouts[0].timeout_ms, 5000);
		assert_eq!(config.timeouts[0].idle_cmd.as_deref(), Some("lock"));
		assert_eq!(config.timeouts[0].resume_cmd.as_deref(), Some("wake"));
	}

	#[test]
	fn parses_fullscreen_policy() {
		let config = parse_args(&args(&[
			"dozed",
			"--fullscreen-policy=ignore",
			"timeout",
			"1",
			"idle",
		]))
		.unwrap();

		assert_eq!(config.fullscreen_policy, FullscreenPolicy::Ignore);
	}

	#[test]
	fn rejects_bad_fullscreen_policy() {
		let err = match parse_args(&args(&["dozed", "--fullscreen-policy", "wat"])) {
			Ok(_) => panic!("expected fullscreen policy parse failure"),
			Err(err) => err,
		};

		assert!(err.contains("invalid fullscreen policy"));
	}

	#[test]
	fn parses_shell_words_with_quotes() {
		let words = shell_words("timeout 5 'notify-send dozed idle' resume \"wake up\"")
			.unwrap();

		assert_eq!(
			words,
			args(&[
				"timeout",
				"5",
				"notify-send dozed idle",
				"resume",
				"wake up",
			])
		);
	}

	#[test]
	fn rejects_unterminated_quote() {
		let err = shell_words("timeout 5 'broken").unwrap_err();

		assert!(err.contains("unterminated"));
	}

	#[test]
	fn rejects_idlehint_as_unknown() {
		let err = match parse_args(&args(&["dozed", "idlehint", "5"])) {
			Ok(_) => panic!("expected idlehint parse failure"),
			Err(err) => err,
		};

		assert_eq!(err, "unsupported command: idlehint");
	}
}

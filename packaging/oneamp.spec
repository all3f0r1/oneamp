%global appid io.github.all3f0r1.OneAmp

# We ship a stripped binary already; skip RPM's debuginfo split (cargo's
# release profile uses LTO + strip and there is little debuginfo left to keep).
%global debug_package %{nil}

Name:           oneamp
Version:        %{?_version}%{!?_version:0.0.0}
Release:        1%{?dist}
Summary:        Pixel-perfect Winamp 2.x in Rust - plays your music, looks good doing it

License:        MIT
URL:            https://github.com/all3f0r1/oneamp
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  gcc
BuildRequires:  pkgconfig
BuildRequires:  rust
BuildRequires:  cargo
BuildRequires:  alsa-lib-devel
BuildRequires:  libxkbcommon-devel
BuildRequires:  wayland-devel
# `dbus-devel` is still required: the raw `dbus` crate is gone, but
# souvlaki's `use_dbus` feature pulls in `dbus-rs` transitively (via
# `dbus-crossroads`) for the MPRIS player, and the dbus-rs build script
# links libdbus-1 via pkg-config.
BuildRequires:  dbus-devel
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

Requires:       alsa-lib
Requires:       libxkbcommon
Requires:       libwayland-client
Requires:       mesa-libEGL
Requires:       mesa-libGL
Requires:       dbus-libs
Recommends:     pipewire-pulseaudio

%description
OneAmp is a native audio player in Rust, faithful to Winamp 2.x - your .wsz
skins render pixel-perfect, your local library plays, internet radio streams,
and the whole thing stays out of your way.

Deliberately not a media library, not a CD ripper, not an iPod sync tool,
not a podcast subscription manager. It's a music player. It plays music.

Decodes MP3, FLAC, OGG Vorbis, WAV, AAC, M4A/MP4, and ALAC via Symphonia +
rodio. Ships a 10-band RBJ-biquad equalizer at ISO 266 octave centres
(constant-Q, click-free, 16 built-in presets, .eqf import/export) with
per-preset auto-preamp so heavy curves don't trip the brickwall limiter.
HTTP/HTTPS internet-radio + podcast streaming with live ICY metadata.
Built-in tag editor (ID3v1/v2, Vorbis, MP4 atoms, RIFF, APE) on the
playlist right-click menu. Customisable playlist row template, gapless +
crossfade, ReplayGain, Fletcher-Munson loudness compensation, MPRIS2 media
bus, multimedia keys, sleep timer.

%prep
%setup -q

%build
# Cargo.lock pins all transitive deps including the onedrop git sources;
# network access is required during the release build to fetch them.
export CARGO_HOME="%{_builddir}/cargo-home"
cargo build --release --locked

%install
install -Dm0755 target/release/oneamp %{buildroot}%{_bindir}/oneamp

install -Dm0644 packaging/%{appid}.desktop \
    %{buildroot}%{_datadir}/applications/%{appid}.desktop
install -Dm0644 packaging/%{appid}.metainfo.xml \
    %{buildroot}%{_datadir}/metainfo/%{appid}.metainfo.xml

for size in 16 32 48 64 128 256 512; do
  install -Dm0644 packaging/icons/oneamp-${size}.png \
    %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/%{appid}.png
done

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{appid}.desktop
appstream-util validate-relax --nonet \
  %{buildroot}%{_datadir}/metainfo/%{appid}.metainfo.xml

%files
%license LICENSE-MIT LICENSE-APACHE
%doc README.md CHANGELOG.md
%{_bindir}/oneamp
%{_datadir}/applications/%{appid}.desktop
%{_datadir}/metainfo/%{appid}.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/%{appid}.png

%changelog
* Wed May 06 2026 OneAmp Project <noreply@github.com> - 0.16.2-1
- Initial RPM packaging. See CHANGELOG.md for the full release history.

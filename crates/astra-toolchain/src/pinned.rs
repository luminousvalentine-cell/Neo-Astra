pub struct PinnedArtifact {
    pub id: &'static str,
    pub version: &'static str,
    pub family: &'static str,
    pub platform: &'static str,
    pub arch: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub format: &'static str,
}

pub const PINNED_ARTIFACTS: &[PinnedArtifact] = &[
    PinnedArtifact {
        id: "zig@0.13.0",
        version: "0.13.0",
        family: "zig",
        platform: "linux",
        arch: "x64",
        url: "https://ziglang.org/download/0.13.0/zig-linux-x86_64-0.13.0.tar.xz",
        sha256: "d45312e61ebcc48032b77bc4cf7fd6915c11fa16e4aad116b66c9468211230ea",
        size: 47_082_308,
        format: "tar.xz",
    },
    PinnedArtifact {
        id: "zig@0.13.0",
        version: "0.13.0",
        family: "zig",
        platform: "linux",
        arch: "arm64",
        url: "https://ziglang.org/download/0.13.0/zig-linux-aarch64-0.13.0.tar.xz",
        sha256: "041ac42323837eb5624068acd8b00cd5777dac4cf91179e8dad7a7e90dd0c556",
        size: 43_090_688,
        format: "tar.xz",
    },
    PinnedArtifact {
        id: "zig@0.13.0",
        version: "0.13.0",
        family: "zig",
        platform: "darwin",
        arch: "x64",
        url: "https://ziglang.org/download/0.13.0/zig-macos-x86_64-0.13.0.tar.xz",
        sha256: "8b06ed1091b2269b700b3b07f8e3be3b833000841bae5aa6a09b1a8b4773effd",
        size: 48_857_012,
        format: "tar.xz",
    },
    PinnedArtifact {
        id: "zig@0.13.0",
        version: "0.13.0",
        family: "zig",
        platform: "darwin",
        arch: "arm64",
        url: "https://ziglang.org/download/0.13.0/zig-macos-aarch64-0.13.0.tar.xz",
        sha256: "46fae219656545dfaf4dce12fb4e8685cec5b51d721beee9389ab4194d43394c",
        size: 44_892_040,
        format: "tar.xz",
    },
    PinnedArtifact {
        id: "zig@0.13.0",
        version: "0.13.0",
        family: "zig",
        platform: "win32",
        arch: "x64",
        url: "https://ziglang.org/download/0.13.0/zig-windows-x86_64-0.13.0.zip",
        sha256: "d859994725ef9402381e557c60bb57497215682e355204d754ee3df75ee3c158",
        size: 79_163_968,
        format: "zip",
    },
    PinnedArtifact {
        id: "zig@0.13.0",
        version: "0.13.0",
        family: "zig",
        platform: "win32",
        arch: "arm64",
        url: "https://ziglang.org/download/0.13.0/zig-windows-aarch64-0.13.0.zip",
        sha256: "95ff88427af7ba2b4f312f45d2377ce7a033e5e3c620c8caaa396a9aba20efda",
        size: 75_119_033,
        format: "zip",
    },
];

pub fn find_pinned(id: &str, platform: &str, arch: &str) -> Option<&'static PinnedArtifact> {
    PINNED_ARTIFACTS
        .iter()
        .find(|a| a.id == id && a.platform == platform && a.arch == arch)
}

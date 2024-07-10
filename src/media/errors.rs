#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum OpenError {
    FileCorrupt,
    UnsupportedFormat,
    Unknown,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum CloseError {
    Unknown,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum PlaybackStartError {
    NothingOpen,
    NothingToPlay,
    Undecodable,
    BrokenContainer,
    ContainerSupportedButNotCodec,
    Unknown,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum PlaybackStopError {
    NothingOpen,
    Unknown,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum PlaybackReadError {
    NothingOpen,
    NeverStarted,
    EOF,
    Unknown,
    DecodeFatal,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum MetadataError {
    NothingOpen,
    Unknown,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum DurationError {
    NothingOpen,
    NeverDecoded,
    Unknown,
}
//! Host-side sources and workers behind the picker reducer in
//! `plexi_host_model::picker`: the Files index, the preview loader and the usage
//! history. They touch the disk, threads and config; the reducer never does.

pub(crate) mod files;
pub(crate) mod preview;
pub(crate) mod usage;

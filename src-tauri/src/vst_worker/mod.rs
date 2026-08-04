//! Isolated VST worker protocol and Windows process supervision.

pub mod protocol;
#[cfg(target_os = "windows")]
pub mod supervisor;

#[allow(unused_imports)]
pub use protocol::{
    monotonic_qpc, qpc_elapsed_us, ControlMessage, MidiWireFrame, WorkerAudioStatus, WorkerMetrics,
    CONTROL_PROTOCOL_VERSION, HOST_ABI_VERSION,
};
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
pub use supervisor::{SupervisorSnapshot, VstWorkerState, VstWorkerSupervisor};

use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_foundation::{NSActivityOptions, NSProcessInfo, ns_string};

/// A foreground editing activity that allows normal system idle sleep.
pub struct LatencyCriticalActivity {
    process_info: Retained<NSProcessInfo>,
    activity: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl LatencyCriticalActivity {
    pub fn begin() -> Self {
        let process_info = NSProcessInfo::processInfo();
        let options = NSActivityOptions::UserInitiatedAllowingIdleSystemSleep
            | NSActivityOptions::LatencyCritical;
        let activity = process_info
            .beginActivityWithOptions_reason(options, ns_string!("Interactive editing"));
        Self { process_info, activity }
    }
}

impl Drop for LatencyCriticalActivity {
    fn drop(&mut self) {
        // SAFETY: This token was returned by beginActivity on this process info object.
        unsafe { self.process_info.endActivity(&self.activity) };
    }
}

//! Offscreen frames in flight. A window's swapchain throttles the CPU to the GPU (acquire
//! waits for a free image), but offscreen rendering (the bench, screenshots) has nothing
//! that does: the CPU queued frames until the kernel's submission queue was full, dozens of
//! frames ahead of the GPU, so a frame's measured time belonged to GPU work from far
//! earlier and the occasional queue drain showed up as 20 ms "submit" spikes. Waiting for
//! the frame from [`MAX_FRAMES`] submissions ago keeps the same depth a swapchain does.

/// Frames the CPU may have queued ahead of the GPU.
const MAX_FRAMES: usize = 2;

#[derive(Default)]
pub(super) struct FramesInFlight {
    queued: std::collections::VecDeque<wgpu::SubmissionIndex>,
}

impl FramesInFlight {
    /// Records this frame's submission and waits for the oldest one past the limit.
    pub(super) fn submitted(&mut self, device: &wgpu::Device, index: wgpu::SubmissionIndex) {
        self.queued.push_back(index);
        while self.queued.len() > MAX_FRAMES {
            let oldest = self.queued.pop_front().unwrap();
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: Some(oldest),
                timeout: None,
            });
        }
    }
}

use eframe::egui;

pub(crate) fn options() -> eframe::NativeOptions {
    let size = std::env::var("GLIMPSE_WINDOW_SIZE")
        .ok()
        .and_then(|value| {
            let (w, h) = value.split_once('x')?;
            Some([
                w.parse::<f32>().ok()?.clamp(520., 2400.),
                h.parse::<f32>().ok()?.clamp(380., 1600.),
            ])
        })
        .unwrap_or([1280., 844.]);
    options_for(size, std::env::var_os("GLIMPSE_BACKGROUND").is_some())
}

fn options_for(size: [f32; 2], background: bool) -> eframe::NativeOptions {
    eframe::NativeOptions {
        // Window inactivity alone does not disable Winit's process-level macOS activation.
        event_loop_builder: background.then(|| {
            Box::new(
                |builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>| {
                    use winit::platform::macos::EventLoopBuilderExtMacOS;
                    builder.with_activate_ignoring_other_apps(false);
                },
            ) as eframe::EventLoopBuilderHook
        }),
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_active(!background)
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(true)
            .with_min_inner_size([520., 380.]),
        // Immediate Metal presentation avoids queuing old scroll positions behind vsync.
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration::default()
            .with_surface_config(surface_config(std::env::var_os("GLIMPSE_VSYNC").is_some())),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    }
}

fn surface_config(vsync: bool) -> eframe::egui_wgpu::SurfaceConfig {
    eframe::egui_wgpu::SurfaceConfig {
        present_mode: if vsync {
            eframe::wgpu::PresentMode::AutoVsync
        } else {
            eframe::wgpu::PresentMode::AutoNoVsync
        },
        desired_maximum_frame_latency: Some(1),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn presentation_uses_low_latency_with_a_vsync_fallback() {
        let responsive = super::surface_config(false);
        assert_eq!(
            responsive.present_mode,
            eframe::wgpu::PresentMode::AutoNoVsync
        );
        assert_eq!(responsive.desired_maximum_frame_latency, Some(1));
        assert_eq!(
            super::surface_config(true).present_mode,
            eframe::wgpu::PresentMode::AutoVsync
        );
    }

    #[test]
    fn background_launch_disables_window_and_process_activation() {
        let background = super::options_for([1280., 844.], true);
        assert_eq!(background.viewport.active, Some(false));
        assert!(background.event_loop_builder.is_some());
        let ordinary = super::options_for([1280., 844.], false);
        assert_eq!(ordinary.viewport.active, Some(true));
        assert!(ordinary.event_loop_builder.is_none());
    }
}

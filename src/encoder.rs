use std::fs::File;
use std::io::Write;
use std::path::Path;

pub trait Encoder {
    fn initialize(&mut self, width: u32, height: u32, fps: u32) -> Result<(), Box<dyn std::error::Error>>;
    fn encode_frame(&mut self, frame_data: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>>;
    #[cfg(target_os = "linux")]
    fn encode_dma_buf(&mut self, fd: std::os::unix::io::RawFd, width: u32, height: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>>;
}

pub struct HardwareEncoder {
    codec: String,
    output_file: Option<File>,
}

impl HardwareEncoder {
    pub fn new(codec: &str, output_path: Option<&Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let output_file = match output_path {
            Some(path) => Some(File::create(path)?),
            None => None,
        };

        Ok(Self {
            codec: codec.to_string(),
            output_file,
        })
    }
}

impl Encoder for HardwareEncoder {
    fn initialize(&mut self, width: u32, height: u32, fps: u32) -> Result<(), Box<dyn std::error::Error>> {
        println!(
            "Initializing hardware encoder with codec: {}, resolution: {}x{}, fps: {}",
            self.codec, width, height, fps
        );
        Ok(())
    }

    fn encode_frame(&mut self, _frame_data: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mock_nal_unit = vec![0, 0, 0, 1, 0x67, 0x64, 0x00, 0x0a, 0xac, 0x72, 0x84, 0x44, 0x26, 0x50];
        
        if let Some(ref mut file) = self.output_file {
            file.write_all(&mock_nal_unit)?;
        }
        
        Ok(mock_nal_unit)
    }

    #[cfg(target_os = "linux")]
    fn encode_dma_buf(&mut self, fd: std::os::unix::io::RawFd, width: u32, height: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        println!("Zero-copy: Encoding DMA-BUF (fd={}) with {}x{} resolution", fd, width, height);
        
        let mock_packet = vec![0, 0, 0, 1, 0x41, 0x9a, 0x1b, 0x22, 0xc0];
        if let Some(ref mut file) = self.output_file {
            file.write_all(&mock_packet)?;
        }
        
        Ok(mock_packet)
    }
}

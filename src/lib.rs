// Capture & encode pipeline
pub mod portal;
pub mod capture;
pub mod encoder;

// Host-side audio & input
pub mod audio;
pub mod input;
pub mod controller;

// Transmission / networking layer
pub mod protocol;
pub mod connection_manager;

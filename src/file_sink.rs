use std::path::Path;

use audiotags::{Tag, TagType};
use librespot::{playback::{audio_backend::{Open, Sink, SinkError}, config::AudioFormat, decoder::AudioPacket, convert::Converter}};

// extern crate flac_bound;

use flac_bound::{FlacEncoder};
use symphonia::{Encoder as Mp3Encoder, StreamConfig as Mp3StreamConfig};

use crate::TrackMetadata;

enum FileType {
    MP3,
    FLAC,
}

pub struct FileSink {
    sink: String,
    content: Vec<i32>,
    metadata: Option<TrackMetadata>,
    file_type: FileType,
}

impl FileSink {
    pub fn add_metadata(&mut self, meta: TrackMetadata) {
        self.metadata = Some(meta);
    }
    pub fn set_file_type(&mut self, file_type: FileType) {
        self.file_type = file_type;
    }
}

impl Open for FileSink {
    fn open(path: Option<String>, _audio_format: AudioFormat) -> Self {
        let file_path = path.unwrap_or_else(|| panic!());
        FileSink {
            sink: file_path,
            content: Vec::new(),
            metadata: None,
            file_type: FileType::FLAC,
        }
    }
}

impl Sink for FileSink {
    fn start(&mut self) -> Result<(), SinkError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), SinkError> {
        match self.file_type {
            FileType::FLAC => {
                let mut encoder = FlacEncoder::new().unwrap().channels(2).bits_per_sample(16).compression_level(4).init_file(&self.sink).unwrap();
                encoder.process_interleaved(self.content.as_slice(), (self.content.len()/2) as u32).unwrap();
                encoder.finish().unwrap();
            },
            FileType::MP3 => {
                let mut encoder = Mp3Encoder::new(&self.sink, Mp3StreamConfig::default()).unwrap();
                encoder.encode(self.content.as_slice()).unwrap();
                encoder.finish().unwrap();
            },
        }

        match &self.metadata {
            Some(meta) => {
                let mut tag = match self.file_type {
                    FileType::FLAC => Tag::new().with_tag_type(TagType::Flac).read_from_path(Path::new(&self.sink)).unwrap(),
                    FileType::MP3 => Tag::new().with_tag_type(TagType::MP3).read_from_path(Path::new(&self.sink)).unwrap(),
                };

                tag.set_album_title(&meta.album);
                for artist in &meta.artists {
                    tag.add_artist(artist);
                }
                tag.set_title(&meta.track_name);
                tag.write_to_path(&self.sink).expect("Failed to write metadata");
            },
            None => (),
        }
        Ok(())
    }

    fn write(&mut self, packet: &AudioPacket, converter: &mut Converter) -> Result<(), SinkError> {
        let data = converter.f64_to_s16(packet.samples().unwrap());
        let mut data32: Vec<i32> = data.iter().map(|el| i32::from(*el)).collect();
        self.content.append(&mut data32);
        Ok(())
    }
}

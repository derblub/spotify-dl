use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use lazy_static::lazy_static;
use librespot::core::session::Session;
use librespot::core::{SpotifyId, SpotifyUri};
use librespot::metadata::Metadata;
use librespot::metadata::image::Image;
use regex::Regex;

use crate::encoder::tags::Tags;
use crate::utils::clean_invalid_characters;

pub type AsyncFn<T> =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<T>> + Send>> + Send + Sync>;

#[async_trait::async_trait]
trait TrackCollection {
    async fn get_tracks(&self, session: &Session) -> Vec<Track>;
}

#[tracing::instrument(name = "get_tracks", skip(session), level = "debug")]
pub async fn get_tracks(spotify_ids: Vec<String>, session: &Session) -> Result<Vec<Track>> {
    let mut tracks: Vec<Track> = Vec::new();
    for id in spotify_ids {
        tracing::debug!("Getting tracks for: {}", id);
        let uri: SpotifyUri = parse_uri_or_url(&id).ok_or(anyhow::anyhow!("Invalid track"))?;
        let new_tracks = match uri {
            SpotifyUri::Track { .. } | SpotifyUri::Episode { .. } => {
                vec![Track { uri: uri.clone(), position: None }]
            }
            SpotifyUri::Album { id } => Album::from_id(id).get_tracks(session).await,
            SpotifyUri::Playlist { id, .. } => Playlist::from_id(id).get_tracks(session).await,
            _ => {
                tracing::warn!("Unsupported item type: {:?}", id);
                vec![]
            }
        };
        tracks.extend(new_tracks);
    }
    tracing::debug!("Got tracks: {:?}", tracks);
    Ok(tracks)
}

fn parse_uri_or_url(track: &str) -> Option<SpotifyUri> {
    parse_uri(track).or_else(|| parse_url(track))
}

fn parse_uri(track_uri: &str) -> Option<SpotifyUri> {
    let res = SpotifyUri::from_uri(track_uri);
    tracing::info!("Parsed URI: {:?}", res);
    res.ok()
}

fn parse_url(track_url: &str) -> Option<SpotifyUri> {
    let results = SPOTIFY_URL_REGEX.captures(track_url)?;
    let uri = format!(
        "spotify:{}:{}",
        results.get(1)?.as_str(),
        results.get(2)?.as_str()
    );
    SpotifyUri::from_uri(&uri).ok()
}

#[derive(Clone, Debug)]
pub struct Track {
    pub uri: SpotifyUri,
    pub position: Option<usize>,
}

lazy_static! {
    static ref SPOTIFY_URL_REGEX: Regex =
        Regex::new(r"https://open\.spotify\.com(?:/intl-[a-z]{2})?/(\w+)/([a-zA-Z0-9]+)").unwrap();
}

impl Track {
    pub fn new(track: &str) -> Result<Self> {
        let uri = parse_uri_or_url(track).ok_or(anyhow::anyhow!("Invalid track"))?;
        Ok(Track { uri, position: None })
    }

    pub async fn metadata(&self, session: &Session) -> Result<TrackMetadata> {
        let metadata = librespot::metadata::Track::get(session, &self.uri)
            .await
            .map_err(|_| anyhow::anyhow!("Failed to get metadata"))?;

        let mut artists = Vec::new();
        for artist in metadata.artists.iter() {
            artists.push(
                librespot::metadata::Artist::get(session, &artist.id)
                    .await
                    .map_err(|_| anyhow::anyhow!("Failed to get artist"))?,
            );
        }

        let album = librespot::metadata::Album::get(session, &metadata.album.id)
            .await
            .map_err(|_| anyhow::anyhow!("Failed to get album"))?;

        let covers = album.covers.clone();
        let session = session.clone();

        let image_retriever: AsyncFn<Bytes> = Arc::new(move || {
            let covers = covers.clone();
            let session = session.clone();

            Box::pin(async move {
                let cover = covers.first()?;
                session.spclient().get_image(&cover.id).await.ok()
            })
        });

        let position = self.position;

        Ok(TrackMetadata::from(
            metadata,
            artists,
            album,
            position,
            image_retriever,
        ))
    }
}

#[async_trait::async_trait]
impl TrackCollection for Track {
    async fn get_tracks(&self, _session: &Session) -> Vec<Track> {
        vec![self.clone()]
    }
}

pub struct Album {
    id: SpotifyId,
}

impl Album {
    pub fn new(album: &str) -> Result<Self> {
        let id = parse_uri_or_url(album)
            .and_then(|uri| match uri {
                SpotifyUri::Album { id } => Some(id),
                _ => None,
            })
            .ok_or(anyhow::anyhow!("Invalid album"))?;
        Ok(Album { id })
    }

    pub fn from_id(id: SpotifyId) -> Self {
        Album { id }
    }

    pub async fn is_album(id: SpotifyUri, session: &Session) -> bool {
        librespot::metadata::Album::get(session, &id).await.is_ok()
    }
}

#[async_trait::async_trait]
impl TrackCollection for Album {
    async fn get_tracks(&self, session: &Session) -> Vec<Track> {
        let uri = SpotifyUri::Album { id: self.id };
        let mut last_err = String::new();
        for attempt in 1..=3 {
            match librespot::metadata::Album::get(session, &uri).await {
                Ok(album) => {
                    return album
                        .tracks()
                        .filter_map(|uri| match uri {
                            SpotifyUri::Track { .. } | SpotifyUri::Episode { .. } => {
                                Some(Track { uri: uri.clone(), position: None })
                            }
                            _ => None,
                        })
                        .collect();
                }
                Err(e) => {
                    last_err = format!("{:?}", e);
                    tracing::warn!("Attempt {}/3 to fetch album failed: {}", attempt, last_err);
                    tokio::time::sleep(std::time::Duration::from_secs(attempt * 3)).await;
                }
            }
        }
        tracing::error!("Failed to get album after 3 attempts: {}", last_err);
        eprintln!("  ⚠ Failed to fetch album: {}", last_err);
        vec![]
    }
}

pub struct Playlist {
    uri: SpotifyUri,
}

impl Playlist {
    pub fn new(playlist: &str) -> Result<Self> {
        let uri = parse_uri_or_url(playlist).ok_or(anyhow::anyhow!("Invalid playlist"))?;
        Ok(Playlist { uri })
    }

    pub fn from_id(id: SpotifyId) -> Self {
        Playlist {
            uri: SpotifyUri::Playlist {
                user: None,
                id,
            },
        }
    }

    pub async fn is_playlist(id: SpotifyUri, session: &Session) -> bool {
        librespot::metadata::Playlist::get(session, &id)
            .await
            .is_ok()
    }
}

#[async_trait::async_trait]
impl TrackCollection for Playlist {
    async fn get_tracks(&self, session: &Session) -> Vec<Track> {
        let mut last_err = String::new();
        for attempt in 1..=3 {
            match librespot::metadata::Playlist::get(session, &self.uri).await {
                Ok(playlist) => {
                    return playlist
                        .tracks()
                        .enumerate()
                        .filter_map(|(i, uri)| match uri {
                            SpotifyUri::Track { .. } | SpotifyUri::Episode { .. } => {
                                Some(Track { uri: uri.clone(), position: Some(i + 1) })
                            }
                            _ => None,
                        })
                        .collect();
                }
                Err(e) => {
                    last_err = format!("{:?}", e);
                    tracing::warn!("Attempt {}/3 to fetch playlist failed: {}", attempt, last_err);
                    tokio::time::sleep(std::time::Duration::from_secs(attempt * 3)).await;
                }
            }
        }
        tracing::error!("Failed to get playlist after 3 attempts: {}", last_err);
        eprintln!("  ⚠ Failed to fetch playlist: {}", last_err);
        vec![]
    }
}

pub struct TrackMetadata {
    pub artists: Vec<ArtistMetadata>,
    pub track_name: String,
    pub album: AlbumMetadata,
    pub duration: i32,
    pub track_number: i32,
    pub disc_number: i32,
    pub position: Option<usize>,
    image_retriever: AsyncFn<Bytes>,
}

impl TrackMetadata {
    pub fn from(
        track: librespot::metadata::Track,
        artists: Vec<librespot::metadata::Artist>,
        album: librespot::metadata::Album,
        position: Option<usize>,
        image_retriever: AsyncFn<Bytes>,
    ) -> Self {
        let artists = artists
            .iter()
            .map(|artist| ArtistMetadata::from(artist.clone()))
            .collect();
        let album = AlbumMetadata::from(album);

        TrackMetadata {
            artists,
            track_name: track.name.clone(),
            album,
            duration: track.duration,
            track_number: track.number,
            disc_number: track.disc_number,
            position,
            image_retriever,
        }
    }

    pub fn approx_size(&self) -> usize {
        let duration = self.duration / 1000;
        let sample_rate = 44100;
        let channels = 2;
        let bits_per_sample = 32;
        let bytes_per_sample = bits_per_sample / 8;
        (duration as usize) * sample_rate * channels * bytes_per_sample
    }

    pub async fn tags(&self) -> Result<Tags> {
        let tags = Tags {
            title: self.track_name.clone(),
            artists: self.artists.iter().map(|a| a.name.clone()).collect(),
            album_title: self.album.name.clone(),
            album_cover: (self.image_retriever)().await,
            track_number: if self.track_number > 0 { Some(self.track_number as u16) } else { None },
            disc_number: if self.disc_number > 0 { Some(self.disc_number as u16) } else { None },
        };
        Ok(tags)
    }
}

impl TrackMetadata {
    /// Returns the filename without any position prefix, e.g. "Artist - Song"
    pub fn to_string_without_position(&self) -> String {
        if self.artists.len() > 3 {
            let artists_name = self
                .artists
                .iter()
                .take(3)
                .map(|artist| artist.name.clone())
                .collect::<Vec<String>>()
                .join(", ");
            return clean_invalid_characters(format!(
                "{}, ... - {}",
                artists_name, self.track_name
            ));
        }

        let artists_name = self
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>()
            .join(", ");
        clean_invalid_characters(format!("{} - {}", artists_name, self.track_name))
    }
}

impl ToString for TrackMetadata {
    fn to_string(&self) -> String {
        let position_prefix = self.position.map_or(String::new(), |i| format!("{:02} - ", i));
        let bare = self.to_string_without_position();
        format!("{}{}", position_prefix, bare)
    }
}

#[derive(Clone, Debug)]
pub struct ArtistMetadata {
    pub name: String,
}

impl From<librespot::metadata::Artist> for ArtistMetadata {
    fn from(artist: librespot::metadata::Artist) -> Self {
        ArtistMetadata {
            name: artist.name.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AlbumMetadata {
    pub name: String,
    pub cover: Option<Image>,
}

impl From<librespot::metadata::Album> for AlbumMetadata {
    fn from(album: librespot::metadata::Album) -> Self {
        AlbumMetadata {
            name: album.name.clone(),
            cover: album.covers.first().cloned(),
        }
    }
}

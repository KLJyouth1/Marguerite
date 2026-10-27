namespace MargueriteDownloader
{
    /// <summary>
    /// The one rule this app exists for: the installer may only open after the
    /// film has played a complete first pass AND the payload has finished
    /// downloading. Whichever finishes last opens the door.
    /// </summary>
    public sealed class FilmGate
    {
        public bool FirstPassComplete { get; private set; }

        public bool DownloadComplete { get; private set; }

        public bool ShouldLaunch
        {
            get { return FirstPassComplete && DownloadComplete; }
        }

        public bool MarkFirstPass()
        {
            FirstPassComplete = true;
            return ShouldLaunch;
        }

        public bool MarkDownload()
        {
            DownloadComplete = true;
            return ShouldLaunch;
        }

        /// <summary>
        /// Used when the film cannot play at all: the gate opens rather than
        /// silently blocking installation behind a broken player.
        /// </summary>
        public void SkipFilm()
        {
            FirstPassComplete = true;
        }
    }
}

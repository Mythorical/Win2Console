// Win2Console Bridge: exports Playnite's library for Win2Console and runs library updates and installs on its
// behalf. Win2Console launches games itself using the play actions exported here.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Windows;
using Playnite.SDK;
using Playnite.SDK.Data;
using Playnite.SDK.Events;
using Playnite.SDK.Models;
using Playnite.SDK.Plugins;

namespace Win2ConsoleBridge
{
    public class Win2ConsoleBridge : GenericPlugin
    {
        private static readonly ILogger logger = LogManager.GetLogger();
        public override Guid Id { get; } = Guid.Parse("0c6a2e1d-5b7f-4c1e-9a3d-7f2b8e4c6d10");

        /// <summary>Set when Playnite was started by Win2Console just to update the library.</summary>
        private bool syncThenExit;
        private readonly DateTime startedAt = DateTime.UtcNow;

        public Win2ConsoleBridge(IPlayniteAPI api) : base(api)
        {
            Properties = new GenericPluginProperties { HasSettings = false };
        }

        private static string ExportPath =>
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "Win2Console", "playnite-library.json");

        public override void OnApplicationStarted(OnApplicationStartedEventArgs args)
        {
            PlayniteApi.UriHandler.RegisterSource("win2console", uri =>
            {
                var a = uri.Arguments ?? new string[0];
                var command = a.Length > 0 ? a[0].ToLowerInvariant() : "";
                try
                {
                    if (command == "sync")
                    {
                        // Only when this request started Playnite: stay hidden while it updates libraries on startup,
                        // then OnLibraryUpdated exports and exits. A Playnite the user already has open is left alone.
                        if (DateTime.UtcNow - startedAt < TimeSpan.FromSeconds(30))
                        {
                            syncThenExit = true;
                            Application.Current?.MainWindow?.Hide();
                        }
                        Export();
                    }
                    else if (command == "export")
                    {
                        Export();
                    }
                    else if ((command == "install" || command == "uninstall") && a.Length > 1 && Guid.TryParse(a[1], out var id))
                    {
                        if (command == "install") PlayniteApi.InstallGame(id); else PlayniteApi.UninstallGame(id);
                    }
                }
                catch (Exception e)
                {
                    logger.Error(e, "Win2Console command failed: " + command);
                }
            });
            Export();
        }

        public override void OnLibraryUpdated(OnLibraryUpdatedEventArgs args)
        {
            Export();
            if (syncThenExit)
            {
                PlayniteApi.MainView.UIDispatcher.BeginInvoke(new Action(() => Application.Current.Shutdown()));
            }
        }

        public override void OnGameInstalled(OnGameInstalledEventArgs args) => Export();
        public override void OnGameUninstalled(OnGameUninstalledEventArgs args) => Export();

        private string FullPath(string dbPath) =>
            string.IsNullOrEmpty(dbPath) ? null : PlayniteApi.Database.GetFullFilePath(dbPath);

        private static long? Unix(DateTime? time) =>
            time.HasValue ? (long?)new DateTimeOffset(time.Value.ToUniversalTime()).ToUnixTimeSeconds() : null;

        /// <summary>The action that starts the game, from the game's own play actions or its library plugin.</summary>
        private Dictionary<string, object> PlayAction(Game game)
        {
            var action = game.GameActions?.FirstOrDefault(x => x.IsPlayAction &&
                (x.Type == GameActionType.File || x.Type == GameActionType.URL));
            if (action != null)
            {
                var expanded = PlayniteApi.ExpandGameVariables(game, action);
                return new Dictionary<string, object>
                {
                    ["kind"] = expanded.Type == GameActionType.URL ? "url" : "file",
                    ["path"] = expanded.Path,
                    ["args"] = expanded.Arguments,
                    ["working_dir"] = expanded.WorkingDir,
                };
            }
            if (!game.IsInstalled) return null;
            var plugin = PlayniteApi.Addons.Plugins.OfType<LibraryPlugin>().FirstOrDefault(p => p.Id == game.PluginId);
            if (plugin == null) return null;
            try
            {
                var controller = plugin.GetPlayActions(new GetPlayActionsArgs { Game = game })?
                    .OfType<AutomaticPlayController>().FirstOrDefault();
                if (controller == null) return null;
                return new Dictionary<string, object>
                {
                    ["kind"] = controller.Type == AutomaticPlayActionType.Url ? "url" : "file",
                    ["path"] = PlayniteApi.ExpandGameVariables(game, controller.Path),
                    ["args"] = PlayniteApi.ExpandGameVariables(game, controller.Arguments),
                    ["working_dir"] = PlayniteApi.ExpandGameVariables(game, controller.WorkingDir),
                    ["tracking_path"] = PlayniteApi.ExpandGameVariables(game, controller.TrackingPath),
                };
            }
            catch (Exception e)
            {
                logger.Warn(e, "Couldn't get play action for " + game.Name);
                return null;
            }
        }

        private void Export()
        {
            try
            {
                var games = PlayniteApi.Database.Games.Select(g => new Dictionary<string, object>
                {
                    ["id"] = g.Id.ToString(),
                    ["name"] = g.Name,
                    ["source"] = g.Source?.Name,
                    ["plugin_id"] = g.PluginId.ToString(),
                    ["game_id"] = g.GameId,
                    ["installed"] = g.IsInstalled,
                    ["hidden"] = g.Hidden,
                    ["install_dir"] = g.InstallDirectory,
                    ["cover"] = FullPath(g.CoverImage),
                    ["background"] = FullPath(g.BackgroundImage),
                    ["icon"] = FullPath(g.Icon),
                    ["playtime"] = g.Playtime,
                    ["last_activity"] = Unix(g.LastActivity),
                    ["play"] = PlayAction(g),
                }).ToList();
                var export = new Dictionary<string, object>
                {
                    ["version"] = 1,
                    ["exported_at"] = Unix(DateTime.UtcNow),
                    ["playnite_exe"] = Path.Combine(PlayniteApi.Paths.ApplicationPath, "Playnite.DesktopApp.exe"),
                    ["games"] = games,
                };
                Directory.CreateDirectory(Path.GetDirectoryName(ExportPath));
                var tmp = ExportPath + ".tmp";
                File.WriteAllText(tmp, Serialization.ToJson(export, true));
                if (File.Exists(ExportPath)) File.Delete(ExportPath);
                File.Move(tmp, ExportPath);
                logger.Info($"Exported {games.Count} games for Win2Console");
            }
            catch (Exception e)
            {
                logger.Error(e, "Win2Console export failed");
            }
        }
    }
}

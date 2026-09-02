#!/usr/bin/env bash

# Dependencies
source $(dirname $0)/support-keep-alive.sh
source $(dirname $0)/support-print.sh
source $(dirname $0)/support-require-sudo.sh

# ------------------------------------------------------------------------------

print::title "Personal Preferences"
print::title_paragraph "This script will apply the personal preferences you like using on macOS."

# ===========================================================================================
# Homebrew permissions

print::command "chmod g-w /opt/homebrew/share"

# ===========================================================================================
# Add Login Items

print::command "osascript -e 'tell application \"System Events\" to make login item at end with properties {path:\"/Applications/Docker.app/\", hidden:true}'"
print::command "osascript -e 'tell application \"System Events\" to make login item at end with properties {path:\"/Applications/Velja.app/\", hidden:true}'"
print::command "osascript -e 'tell application \"System Events\" to make login item at end with properties {path:\"/Applications/Macs Fan Control.app\", hidden:false}'"
print::command "osascript -e 'tell application \"System Events\" to make login item at end with properties {path:\"/Applications/Alfred 5.app\", hidden:false}'"
print::command "osascript -e 'tell application \"System Events\" to make login item at end with properties {path:\"/Applications/Magnet.app\", hidden:false}'"

# Based on https://github.com/mathiasbynens/dotfiles/blob/master/.macos

# ===========================================================================================

# Close any open System Preferences panes, to prevent them from overriding
# settings we’re about to change
print::command "osascript -e 'tell application \"System Settings\" to quit'"

# Keep-alive: update existing `sudo` time stamp until `.macos` has finished
print::command "while true; do sudo -n true; sleep 60; kill -0 "$$" || exit; done 2>/dev/null &"

###############################################################################
# General UI/UX                                                               #
###############################################################################

# Set computer name (as done via System Preferences → Sharing)
# print::command "sudo scutil --set ComputerName "0x6D746873""
# print::command "sudo scutil --set HostName "0x6D746873""
# print::command "sudo scutil --set LocalHostName "0x6D746873""
# print::command "sudo defaults write /Library/Preferences/SystemConfiguration/com.apple.smb.server NetBIOSName -string "0x6D746873""

# Set standby delay to 24 hours (default is 1 hour)
# print::command "sudo pmset -a standbydelay 86400"

# Disable the sound effects on boot
print::command "sudo nvram SystemAudioVolume=\" \""

# Disable transparency in the menu bar and elsewhere on Yosemite
# print::command "defaults write com.apple.universalaccess reduceTransparency -bool true"

# Set highlight color to green
print::command "defaults write NSGlobalDomain AppleHighlightColor -string \"0.764700 0.976500 0.568600\""

# Set sidebar icon size to medium
print::command "defaults write NSGlobalDomain NSTableViewDefaultSizeMode -int 2"

# Always show scrollbars
print::command "defaults write NSGlobalDomain AppleShowScrollBars -string \"Always\""
# Possible values: `WhenScrolling`, `Automatic` and `Always`

# Disable the over-the-top focus ring animation
print::command "defaults write NSGlobalDomain NSUseAnimatedFocusRing -bool false"

# Disable smooth scrolling
# (Uncomment if you’re on an older Mac that messes up the animation)
# print::command "defaults write NSGlobalDomain NSScrollAnimationEnabled -bool false"

# Increase window resize speed for Cocoa applications
print::command "defaults write NSGlobalDomain NSWindowResizeTime -float 0.001"

# Expand save panel by default
print::command "defaults write NSGlobalDomain NSNavPanelExpandedStateForSaveMode -bool true"
print::command "defaults write NSGlobalDomain NSNavPanelExpandedStateForSaveMode2 -bool true"

# Expand print panel by default
print::command "defaults write NSGlobalDomain PMPrintingExpandedStateForPrint -bool true"
print::command "defaults write NSGlobalDomain PMPrintingExpandedStateForPrint2 -bool true"

# Save to disk (not to iCloud) by default
# print::command "defaults write NSGlobalDomain NSDocumentSaveNewDocumentsToCloud -bool false"

# Automatically quit printer app once the print jobs complete
print::command "defaults write com.apple.print.PrintingPrefs \"Quit When Finished\" -bool true"

# Disable the “Are you sure you want to open this application?” dialog
print::command "defaults write com.apple.LaunchServices LSQuarantine -bool false"

# Remove duplicates in the “Open With” menu (also see `lscleanup` alias)
print::command "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -kill -r -domain local -domain system -domain user"

# Display ASCII control characters using caret notation in standard text views
# Try e.g. `cd /tmp; unidecode "\x{0000}" > cc.txt; open -e cc.txt`
print::command "defaults write NSGlobalDomain NSTextShowsControlCharacters -bool true"

# Disable Resume system-wide
print::command "defaults write com.apple.systempreferences NSQuitAlwaysKeepsWindows -bool false"

# Disable automatic termination of inactive apps
print::command "defaults write NSGlobalDomain NSDisableAutomaticTermination -bool true"

# Disable the crash reporter
print::command "defaults write com.apple.CrashReporter DialogType -string \"none\""

# Set Help Viewer windows to non-floating mode
print::command "defaults write com.apple.helpviewer DevMode -bool true"

# Fix for the ancient UTF-8 bug in QuickLook (https://mths.be/bbo)
# Commented out, as this is known to cause problems in various Adobe apps :(
# See https://github.com/mathiasbynens/dotfiles/issues/237
#echo "0x08000100:0" > ~/.CFUserTextEncoding

# Reveal IP address, hostname, OS version, etc. when clicking the clock
# in the login window
print::command "sudo defaults write /Library/Preferences/com.apple.loginwindow AdminHostInfo HostName"

# Restart automatically if the computer freezes
print::command "sudo systemsetup -setrestartfreeze on"

# Never go into computer sleep mode
# print::command "sudo systemsetup -setcomputersleep Off > /dev/null"

# Disable Notification Center and remove the menu bar icon
# launchctl unload -w /System/Library/LaunchAgents/com.apple.notificationcenterui.plist 2> /dev/null

# Disable automatic capitalization as it’s annoying when typing code
print::command "defaults write NSGlobalDomain NSAutomaticCapitalizationEnabled -bool false"

# Disable smart dashes as they’re annoying when typing code
print::command "defaults write NSGlobalDomain NSAutomaticDashSubstitutionEnabled -bool false"

# Disable automatic period substitution as it’s annoying when typing code
print::command "defaults write NSGlobalDomain NSAutomaticPeriodSubstitutionEnabled -bool false"

# Disable smart quotes as they’re annoying when typing code
print::command "defaults write NSGlobalDomain NSAutomaticQuoteSubstitutionEnabled -bool false"

# Disable auto-correct
print::command "defaults write NSGlobalDomain NSAutomaticSpellingCorrectionEnabled -bool false"

# Set a custom wallpaper image. `DefaultDesktop.jpg` is already a symlink, and
# all wallpapers are in `/Library/Desktop Pictures/`. The default is `Wave.jpg`.
#rm -rf ~/Library/Application Support/Dock/desktoppicture.db
# print::command "sudo rm -rf /System/Library/CoreServices/DefaultDesktop.jpg"
# print::command "sudo ln -s /path/to/your/image /System/Library/CoreServices/DefaultDesktop.jpg"

###############################################################################
# SSD-specific tweaks                                                         #
###############################################################################

# Disable hibernation (speeds up entering sleep mode)
# print::command "sudo pmset -a hibernatemode 0"

# Remove the sleep image file to save disk space
# print::command "sudo rm /private/var/vm/sleepimage"

# Create a zero-byte file instead…
# print::command "sudo touch /private/var/vm/sleepimage"

# …and make sure it can’t be rewritten
# print::command "sudo chflags uchg /private/var/vm/sleepimage"


###############################################################################
# Trackpad, mouse, keyboard, Bluetooth accessories, and input                 #
###############################################################################

# Trackpad: enable tap to click for this user and for the login screen
print::command "defaults write com.apple.driver.AppleBluetoothMultitouch.trackpad Clicking -bool true"
print::command "defaults -currentHost write NSGlobalDomain com.apple.mouse.tapBehavior -int 1"
print::command "defaults write NSGlobalDomain com.apple.mouse.tapBehavior -int 1"

# Trackpad: map bottom right corner to right-click
print::command "defaults write com.apple.driver.AppleBluetoothMultitouch.trackpad TrackpadCornerSecondaryClick -int 2"
print::command "defaults write com.apple.driver.AppleBluetoothMultitouch.trackpad TrackpadRightClick -bool true"
print::command "defaults -currentHost write NSGlobalDomain com.apple.trackpad.trackpadCornerClickBehavior -int 1"
print::command "defaults -currentHost write NSGlobalDomain com.apple.trackpad.enableSecondaryClick -bool true"

# Disable “natural” (Lion-style) scrolling
# print::command "defaults write NSGlobalDomain com.apple.swipescrolldirection -bool false"

# Increase sound quality for Bluetooth headphones/headsets
print::command "defaults write com.apple.BluetoothAudioAgent \"Apple Bitpool Min (editable)\" -int 40"

# Enable full keyboard access for all controls
# (e.g. enable Tab in modal dialogs)
print::command "defaults write NSGlobalDomain AppleKeyboardUIMode -int 3"

# Use scroll gesture with the Ctrl (^) modifier key to zoom
print::command "sudo defaults write com.apple.universalaccess closeViewScrollWheelToggle -bool true"
print::command "sudo defaults write com.apple.universalaccess HIDScrollZoomModifierMask -int 262144"
# Follow the keyboard focus while zoomed in
print::command "sudo defaults write com.apple.universalaccess closeViewZoomFollowsFocus -bool true"

# Disable press-and-hold for keys in favor of key repeat
print::command "defaults write NSGlobalDomain ApplePressAndHoldEnabled -bool false"

# Set a blazingly fast keyboard repeat rate
print::command "defaults write NSGlobalDomain KeyRepeat -int 1"
print::command "defaults write NSGlobalDomain InitialKeyRepeat -int 10"

# Set language and text formats
# Note: if you’re in the US, replace `EUR` with `USD`, `Centimeters` with
# `Inches`, `en_GB` with `en_US`, and `true` with `false`.
print::command "defaults write NSGlobalDomain AppleLanguages -array \"en\" \"pt_BR\""
print::command "defaults write NSGlobalDomain AppleLocale -string \"en_GB@currency=BRL\""
print::command "defaults write NSGlobalDomain AppleMeasurementUnits -string \"Centimeters\""
print::command "defaults write NSGlobalDomain AppleMetricUnits -bool true"

# Show language menu in the top right corner of the boot screen
print::command "sudo defaults write /Library/Preferences/com.apple.loginwindow showInputMenu -bool true"

# Set the timezone; see `sudo systemsetup -listtimezones` for other values
print::command "sudo systemsetup -settimezone \"America/Sao_Paulo\" > /dev/null"

# Stop iTunes from responding to the keyboard media keys
#launchctl unload -w /System/Library/LaunchAgents/com.apple.rcd.plist 2> /dev/null

###############################################################################
# Screen                                                                      #
###############################################################################

# Require password immediately after sleep or screen saver begins
print::command "defaults write com.apple.screensaver askForPassword -int 1"
print::command "defaults write com.apple.screensaver askForPasswordDelay -int 0"

# Save screenshots to the desktop
print::command "defaults write com.apple.screencapture location -string \"${HOME}/Desktop\""

# Save screenshots in PNG format (other options: BMP, GIF, JPG, PDF, TIFF)
print::command "defaults write com.apple.screencapture type -string \"png\""

# Disable shadow in screenshots
print::command "defaults write com.apple.screencapture disable-shadow -bool true"

# Enable subpixel font rendering on non-Apple LCDs
# Reference: https://github.com/kevinSuttle/macOS-Defaults/issues/17#issuecomment-266633501
print::command "defaults write NSGlobalDomain AppleFontSmoothing -int 1"

# Enable HiDPI display modes (requires restart)
print::command "sudo defaults write /Library/Preferences/com.apple.windowserver DisplayResolutionEnabled -bool true"

###############################################################################
# Finder                                                                      #
###############################################################################

# Finder: allow quitting via ⌘ + Q; doing so will also hide desktop icons
# print::command "defaults write com.apple.finder QuitMenuItem -bool true"

# Finder: disable window animations and Get Info animations
print::command "defaults write com.apple.finder DisableAllAnimations -bool true"

# Set Desktop as the default location for new Finder windows
# For other paths, use `PfLo` and `file:///full/path/here/`
print::command "defaults write com.apple.finder NewWindowTarget -string \"PfDe\""
print::command "defaults write com.apple.finder NewWindowTargetPath -string \"file://${HOME}/Desktop/\""

# Show icons for hard drives, servers, and removable media on the desktop
print::command "defaults write com.apple.finder ShowExternalHardDrivesOnDesktop -bool true"
print::command "defaults write com.apple.finder ShowHardDrivesOnDesktop -bool true"
print::command "defaults write com.apple.finder ShowMountedServersOnDesktop -bool true"
print::command "defaults write com.apple.finder ShowRemovableMediaOnDesktop -bool true"

# Finder: show hidden files by default
print::command "defaults write com.apple.finder AppleShowAllFiles -bool true"

# Finder: show all filename extensions
print::command "defaults write NSGlobalDomain AppleShowAllExtensions -bool true"

# Finder: show status bar
print::command "defaults write com.apple.finder ShowStatusBar -bool true"

# Finder: show path bar
print::command "defaults write com.apple.finder ShowPathbar -bool true"

# Display full POSIX path as Finder window title
print::command "defaults write com.apple.finder _FXShowPosixPathInTitle -bool true"

# Keep folders on top when sorting by name
print::command "defaults write com.apple.finder _FXSortFoldersFirst -bool true"

# When performing a search, search the current folder by default
print::command "defaults write com.apple.finder FXDefaultSearchScope -string \"SCcf\""

# Disable the warning when changing a file extension
print::command "defaults write com.apple.finder FXEnableExtensionChangeWarning -bool false"

# Enable spring loading for directories
print::command "defaults write NSGlobalDomain com.apple.springing.enabled -bool true"

# Remove the spring loading delay for directories
print::command "defaults write NSGlobalDomain com.apple.springing.delay -float 0"

# Avoid creating .DS_Store files on network or USB volumes
print::command "defaults write com.apple.desktopservices DSDontWriteNetworkStores -bool true"
print::command "defaults write com.apple.desktopservices DSDontWriteUSBStores -bool true"

# Disable disk image verification
# print::command "defaults write com.apple.frameworks.diskimages skip-verify -bool true"
# print::command "defaults write com.apple.frameworks.diskimages skip-verify-locked -bool true"
# print::command "defaults write com.apple.frameworks.diskimages skip-verify-remote -bool true"

# Automatically open a new Finder window when a volume is mounted
print::command "defaults write com.apple.frameworks.diskimages auto-open-ro-root -bool true"
print::command "defaults write com.apple.frameworks.diskimages auto-open-rw-root -bool true"
print::command "defaults write com.apple.finder OpenWindowForNewRemovableDisk -bool true"

# Show item info near icons on the desktop and in other icon views
print::command "/usr/libexec/PlistBuddy -c \"Set :DesktopViewSettings:IconViewSettings:showItemInfo true\" ~/Library/Preferences/com.apple.finder.plist"
print::command "/usr/libexec/PlistBuddy -c \"Set :FK_StandardViewSettings:IconViewSettings:showItemInfo true\" ~/Library/Preferences/com.apple.finder.plist"
print::command "/usr/libexec/PlistBuddy -c \"Set :StandardViewSettings:IconViewSettings:showItemInfo true\" ~/Library/Preferences/com.apple.finder.plist"

# Show item info to the right of the icons on the desktop
print::command "/usr/libexec/PlistBuddy -c \"Set DesktopViewSettings:IconViewSettings:labelOnBottom false\" ~/Library/Preferences/com.apple.finder.plist"

# Enable snap-to-grid for icons on the desktop and in other icon views
print::command "/usr/libexec/PlistBuddy -c \"Set :DesktopViewSettings:IconViewSettings:arrangeBy grid\" ~/Library/Preferences/com.apple.finder.plist"
print::command "/usr/libexec/PlistBuddy -c \"Set :FK_StandardViewSettings:IconViewSettings:arrangeBy grid\" ~/Library/Preferences/com.apple.finder.plist"
print::command "/usr/libexec/PlistBuddy -c \"Set :StandardViewSettings:IconViewSettings:arrangeBy grid\" ~/Library/Preferences/com.apple.finder.plist"

# Increase grid spacing for icons on the desktop and in other icon views
print::command "/usr/libexec/PlistBuddy -c \"Set :DesktopViewSettings:IconViewSettings:gridSpacing 100\" ~/Library/Preferences/com.apple.finder.plist"
print::command "/usr/libexec/PlistBuddy -c \"Set :FK_StandardViewSettings:IconViewSettings:gridSpacing 100\" ~/Library/Preferences/com.apple.finder.plist"
print::command "/usr/libexec/PlistBuddy -c \"Set :StandardViewSettings:IconViewSettings:gridSpacing 100\" ~/Library/Preferences/com.apple.finder.plist"

# Increase the size of icons on the desktop and in other icon views
print::command "/usr/libexec/PlistBuddy -c \"Set :DesktopViewSettings:IconViewSettings:iconSize 80\" ~/Library/Preferences/com.apple.finder.plist"
print::command "/usr/libexec/PlistBuddy -c \"Set :FK_StandardViewSettings:IconViewSettings:iconSize 80\" ~/Library/Preferences/com.apple.finder.plist"
print::command "/usr/libexec/PlistBuddy -c \"Set :StandardViewSettings:IconViewSettings:iconSize 80\" ~/Library/Preferences/com.apple.finder.plist"

# Use list view in all Finder windows by default
# Four-letter codes for the other view modes: `icnv`, `clmv`, `Flwv`
print::command "defaults write com.apple.finder FXPreferredViewStyle -string \"Nlsv\""

# Disable the warning before emptying the Trash
# print::command "defaults write com.apple.finder WarnOnEmptyTrash -bool false"

# Enable AirDrop over Ethernet and on unsupported Macs running Lion
print::command "defaults write com.apple.NetworkBrowser BrowseAllInterfaces -bool true"

# Show the ~/Library folder
print::command "chflags nohidden ~/Library"

# Show the /Volumes folder
print::command "sudo chflags nohidden /Volumes"

# Remove Dropbox’s green checkmark icons in Finder
# file=/Applications/Dropbox.app/Contents/Resources/emblem-dropbox-uptodate.icns
# [ -e "${file}" ] && mv -f "${file}" "${file}.bak"

# Expand the following File Info panes:
# “General”, “Open with”, and “Sharing & Permissions”
print::command "defaults write com.apple.finder FXInfoPanesExpanded -dict \
	General -bool true \
	OpenWith -bool true \
	Privileges -bool true"

###############################################################################
# Dock, Dashboard, and hot corners                                            #
###############################################################################

# Enable highlight hover effect for the grid view of a stack (Dock)
print::command "defaults write com.apple.dock mouse-over-hilite-stack -bool true"

# Set the icon size of Dock items to 36 pixels
print::command "defaults write com.apple.dock tilesize -int 36"

# Change minimize/maximize window effect
print::command "defaults write com.apple.dock mineffect -string \"scale\""

# Minimize windows into their application’s icon
print::command "defaults write com.apple.dock minimize-to-application -bool true"

# Enable spring loading for all Dock items
print::command "defaults write com.apple.dock enable-spring-load-actions-on-all-items -bool true"

# Show indicator lights for open applications in the Dock
print::command "defaults write com.apple.dock show-process-indicators -bool true"

# Wipe all (default) app icons from the Dock
# This is only really useful when setting up a new Mac, or if you don’t use
# the Dock to launch apps.
# print::command "defaults write com.apple.dock persistent-apps -array"

# Show only open applications in the Dock
# print::command "defaults write com.apple.dock static-only -bool true"

# Don’t animate opening applications from the Dock
print::command "defaults write com.apple.dock launchanim -bool false"

# Speed up Mission Control animations
print::command "defaults write com.apple.dock expose-animation-duration -float 0.1"

# Don’t group windows by application in Mission Control
# (i.e. use the old Exposé behavior instead)
print::command "defaults write com.apple.dock expose-group-by-app -bool false"

# Disable Dashboard
print::command "defaults write com.apple.dashboard mcx-disabled -bool true"

# Don’t show Dashboard as a Space
print::command "defaults write com.apple.dock dashboard-in-overlay -bool true"

# Don’t automatically rearrange Spaces based on most recent use
print::command "defaults write com.apple.dock mru-spaces -bool false"

# Remove the auto-hiding Dock delay
print::command "defaults write com.apple.dock autohide-delay -float 0"

# Remove the animation when hiding/showing the Dock
print::command "defaults write com.apple.dock autohide-time-modifier -float 0"

# Automatically hide and show the Dock
print::command "defaults write com.apple.dock autohide -bool false"

# Make Dock icons of hidden applications translucent
print::command "defaults write com.apple.dock showhidden -bool true"

# Don’t show recent applications in Dock
print::command "defaults write com.apple.dock show-recents -bool false"

# Disable the Launchpad gesture (pinch with thumb and three fingers)
# print::command "defaults write com.apple.dock showLaunchpadGestureEnabled -int 0"

# Reset Launchpad, but keep the desktop wallpaper intact
print::command "find \"${HOME}/Library/Application Support/Dock\" -name \"*-*.db\" -maxdepth 1 -delete"

# Add iOS & Watch Simulator to Launchpad
# print::command "sudo ln -sf "/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app" "/Applications/Simulator.app""
# print::command "sudo ln -sf "/Applications/Xcode.app/Contents/Developer/Applications/Simulator (Watch).app" "/Applications/Simulator (Watch).app""

# Add a spacer to the left side of the Dock (where the applications are)
# print::command "defaults write com.apple.dock persistent-apps -array-add '{tile-data={}; tile-type="spacer-tile";}'"
# Add a spacer to the right side of the Dock (where the Trash is)
# print::command "defaults write com.apple.dock persistent-others -array-add '{tile-data={}; tile-type="spacer-tile";}'"

# Hot corners
# Possible values:
#  0: no-op
#  2: Mission Control
#  3: Show application windows
#  4: Desktop
#  5: Start screen saver
#  6: Disable screen saver
#  7: Dashboard
# 10: Put display to sleep
# 11: Launchpad
# 12: Notification Center
# Top left screen corner → Mission Control
print::command "defaults write com.apple.dock wvous-tl-corner -int 2"
print::command "defaults write com.apple.dock wvous-tl-modifier -int 0"
# Top right screen corner → Desktop
print::command "defaults write com.apple.dock wvous-tr-corner -int 4"
print::command "defaults write com.apple.dock wvous-tr-modifier -int 0"
# Bottom left screen corner → Start screen saver
print::command "defaults write com.apple.dock wvous-bl-corner -int 11"
print::command "defaults write com.apple.dock wvous-bl-modifier -int 0"
# Bottom right screen corner → Start screen saver
print::command "defaults write com.apple.dock wvous-br-corner -int 11"
print::command "defaults write com.apple.dock wvous-br-modifier -int 0"

###############################################################################
# Safari & WebKit                                                             #
###############################################################################

# Privacy: don’t send search queries to Apple
print::command "defaults write com.apple.Safari UniversalSearchEnabled -bool false"
print::command "defaults write com.apple.Safari SuppressSearchSuggestions -bool true"

# Press Tab to highlight each item on a web page
print::command "defaults write com.apple.Safari WebKitTabToLinksPreferenceKey -bool true"
print::command "defaults write com.apple.Safari com.apple.Safari.ContentPageGroupIdentifier.WebKit2TabsToLinks -bool true"

# Show the full URL in the address bar (note: this still hides the scheme)
print::command "defaults write com.apple.Safari ShowFullURLInSmartSearchField -bool true"

# Set Safari’s home page to `about:blank` for faster loading
print::command "defaults write com.apple.Safari HomePage -string \"about:blank\""

# Prevent Safari from opening ‘safe’ files automatically after downloading
print::command "defaults write com.apple.Safari AutoOpenSafeDownloads -bool false"

# Allow hitting the Backspace key to go to the previous page in history
print::command "defaults write com.apple.Safari com.apple.Safari.ContentPageGroupIdentifier.WebKit2BackspaceKeyNavigationEnabled -bool true"

# Hide Safari’s bookmarks bar by default
print::command "defaults write com.apple.Safari ShowFavoritesBar -bool false"

# Hide Safari’s sidebar in Top Sites
print::command "defaults write com.apple.Safari ShowSidebarInTopSites -bool false"

# Disable Safari’s thumbnail cache for History and Top Sites
print::command "defaults write com.apple.Safari DebugSnapshotsUpdatePolicy -int 2"

# Enable Safari’s debug menu
print::command "defaults write com.apple.Safari IncludeInternalDebugMenu -bool true"

# Make Safari’s search banners default to Contains instead of Starts With
print::command "defaults write com.apple.Safari FindOnPageMatchesWordStartsOnly -bool false"

# Remove useless icons from Safari’s bookmarks bar
print::command "defaults write com.apple.Safari ProxiesInBookmarksBar \"()\""

# Enable the Develop menu and the Web Inspector in Safari
print::command "defaults write com.apple.Safari IncludeDevelopMenu -bool true"
print::command "defaults write com.apple.Safari WebKitDeveloperExtrasEnabledPreferenceKey -bool true"
print::command "defaults write com.apple.Safari com.apple.Safari.ContentPageGroupIdentifier.WebKit2DeveloperExtrasEnabled -bool true"

# Add a context menu item for showing the Web Inspector in web views
print::command "defaults write NSGlobalDomain WebKitDeveloperExtras -bool true"

# Enable continuous spellchecking
print::command "defaults write com.apple.Safari WebContinuousSpellCheckingEnabled -bool true"
# Disable auto-correct
print::command "defaults write com.apple.Safari WebAutomaticSpellingCorrectionEnabled -bool false"

# Disable AutoFill
print::command "defaults write com.apple.Safari AutoFillFromAddressBook -bool false"
print::command "defaults write com.apple.Safari AutoFillPasswords -bool false"
print::command "defaults write com.apple.Safari AutoFillCreditCardData -bool false"
print::command "defaults write com.apple.Safari AutoFillMiscellaneousForms -bool false"

# Warn about fraudulent websites
print::command "defaults write com.apple.Safari WarnAboutFraudulentWebsites -bool true"

# Disable plug-ins
print::command "defaults write com.apple.Safari WebKitPluginsEnabled -bool false"
print::command "defaults write com.apple.Safari com.apple.Safari.ContentPageGroupIdentifier.WebKit2PluginsEnabled -bool false"

# Disable Java
print::command "defaults write com.apple.Safari WebKitJavaEnabled -bool false"
print::command "defaults write com.apple.Safari com.apple.Safari.ContentPageGroupIdentifier.WebKit2JavaEnabled -bool false"

# Block pop-up windows
print::command "defaults write com.apple.Safari WebKitJavaScriptCanOpenWindowsAutomatically -bool false"
print::command "defaults write com.apple.Safari com.apple.Safari.ContentPageGroupIdentifier.WebKit2JavaScriptCanOpenWindowsAutomatically -bool false"

# Disable auto-playing video
# print::command "defaults write com.apple.Safari WebKitMediaPlaybackAllowsInline -bool false"
# print::command "defaults write com.apple.SafariTechnologyPreview WebKitMediaPlaybackAllowsInline -bool false"
# print::command "defaults write com.apple.Safari com.apple.Safari.ContentPageGroupIdentifier.WebKit2AllowsInlineMediaPlayback -bool false"
# print::command "defaults write com.apple.SafariTechnologyPreview com.apple.Safari.ContentPageGroupIdentifier.WebKit2AllowsInlineMediaPlayback -bool false"

# Enable “Do Not Track”
print::command "defaults write com.apple.Safari SendDoNotTrackHTTPHeader -bool true"

# Update extensions automatically
print::command "defaults write com.apple.Safari InstallExtensionUpdatesAutomatically -bool true"

###############################################################################
# Mail                                                                        #
###############################################################################

# Disable send and reply animations in Mail.app
print::command "defaults write com.apple.mail DisableReplyAnimations -bool true"
print::command "defaults write com.apple.mail DisableSendAnimations -bool true"

# Copy email addresses as `foo@example.com` instead of `Foo Bar <foo@example.com>` in Mail.app
print::command "defaults write com.apple.mail AddressesIncludeNameOnPasteboard -bool false"

# Add the keyboard shortcut ⌘ + Enter to send an email in Mail.app
print::command "defaults write com.apple.mail NSUserKeyEquivalents -dict-add \"Send\" \"@\U21a9\""

# Display emails in threaded mode, sorted by date (oldest at the top)
print::command "defaults write com.apple.mail DraftsViewerAttributes -dict-add \"DisplayInThreadedMode\" -string \"yes\""
print::command "defaults write com.apple.mail DraftsViewerAttributes -dict-add \"SortedDescending\" -string \"yes\""
print::command "defaults write com.apple.mail DraftsViewerAttributes -dict-add \"SortOrder\" -string \"received-date\""

# Disable inline attachments (just show the icons)
print::command "defaults write com.apple.mail DisableInlineAttachmentViewing -bool true"

# Disable automatic spell checking
print::command "defaults write com.apple.mail SpellCheckingBehavior -string \"NoSpellCheckingEnabled\""

###############################################################################
# Spotlight                                                                   #
###############################################################################

# Hide Spotlight tray-icon (and subsequent helper)
# print::command "sudo chmod 600 /System/Library/CoreServices/Search.bundle/Contents/MacOS/Search"
# Disable Spotlight indexing for any volume that gets mounted and has not yet
# been indexed before.
# Use `sudo mdutil -i off "/Volumes/foo"` to stop indexing any volume.
print::command "sudo defaults write .Spotlight-V100/VolumeConfiguration Exclusions -array \"/Volumes\""

# Change indexing order and disable some search results
# Yosemite-specific search results (remove them if you are using macOS 10.9 or older):
# 	MENU_DEFINITION
# 	MENU_CONVERSION
# 	MENU_EXPRESSION
# 	MENU_SPOTLIGHT_SUGGESTIONS (send search queries to Apple)
# 	MENU_WEBSEARCH             (send search queries to Apple)
# 	MENU_OTHER
print::command "defaults write com.apple.spotlight orderedItems -array \
	'{\"enabled\" = 1;\"name\" = \"APPLICATIONS\";}' \
	'{\"enabled\" = 1;\"name\" = \"SYSTEM_PREFS\";}' \
	'{\"enabled\" = 1;\"name\" = \"DIRECTORIES\";}' \
	'{\"enabled\" = 1;\"name\" = \"PDF\";}' \
	'{\"enabled\" = 1;\"name\" = \"FONTS\";}' \
	'{\"enabled\" = 0;\"name\" = \"DOCUMENTS\";}' \
	'{\"enabled\" = 0;\"name\" = \"MESSAGES\";}' \
	'{\"enabled\" = 0;\"name\" = \"CONTACT\";}' \
	'{\"enabled\" = 0;\"name\" = \"EVENT_TODO\";}' \
	'{\"enabled\" = 0;\"name\" = \"IMAGES\";}' \
	'{\"enabled\" = 0;\"name\" = \"BOOKMARKS\";}' \
	'{\"enabled\" = 0;\"name\" = \"MUSIC\";}' \
	'{\"enabled\" = 0;\"name\" = \"MOVIES\";}' \
	'{\"enabled\" = 0;\"name\" = \"PRESENTATIONS\";}' \
	'{\"enabled\" = 0;\"name\" = \"SPREADSHEETS\";}' \
	'{\"enabled\" = 0;\"name\" = \"SOURCE\";}' \
	'{\"enabled\" = 0;\"name\" = \"MENU_DEFINITION\";}' \
	'{\"enabled\" = 0;\"name\" = \"MENU_OTHER\";}' \
	'{\"enabled\" = 0;\"name\" = \"MENU_CONVERSION\";}' \
	'{\"enabled\" = 0;\"name\" = \"MENU_EXPRESSION\";}' \
	'{\"enabled\" = 0;\"name\" = \"MENU_WEBSEARCH\";}' \
	'{\"enabled\" = 0;\"name\" = \"MENU_SPOTLIGHT_SUGGESTIONS\";}'"

# Load new settings before rebuilding the index
print::command "sudo killall mds > /dev/null 2>&1"

# Make sure indexing is enabled for the main volume
print::command "sudo mdutil -i on / > /dev/null"

# Rebuild the index from scratch
print::command "sudo mdutil -E / > /dev/null"

###############################################################################
# Time Machine                                                                #
###############################################################################

# Prevent Time Machine from prompting to use new hard drives as backup volume
print::command "defaults write com.apple.TimeMachine DoNotOfferNewDisksForBackup -bool true"

# Disable local Time Machine backups
# print::command "hash tmutil &> /dev/null && sudo tmutil disablelocal"

###############################################################################
# Activity Monitor                                                            #
###############################################################################

# Show the main window when launching Activity Monitor
print::command "defaults write com.apple.ActivityMonitor OpenMainWindow -bool true"

# Visualize CPU usage in the Activity Monitor Dock icon
print::command "defaults write com.apple.ActivityMonitor IconType -int 5"

# Show all processes in Activity Monitor
print::command "defaults write com.apple.ActivityMonitor ShowCategory -int 0"

# Sort Activity Monitor results by CPU usage
print::command "defaults write com.apple.ActivityMonitor SortColumn -string \"CPUUsage\""
print::command "defaults write com.apple.ActivityMonitor SortDirection -int 0"

###############################################################################
# Address Book, Dashboard, iCal, TextEdit, and Disk Utility                   #
###############################################################################

# Enable the debug menu in Address Book
print::command "defaults write com.apple.addressbook ABShowDebugMenu -bool true"

# Enable Dashboard dev mode (allows keeping widgets on the desktop)
print::command "defaults write com.apple.dashboard devmode -bool true"

# Enable the debug menu in iCal (pre-10.8)
print::command "defaults write com.apple.iCal IncludeDebugMenu -bool true"

# Use plain text mode for new TextEdit documents
print::command "defaults write com.apple.TextEdit RichText -int 0"

# Open and save files as UTF-8 in TextEdit
print::command "defaults write com.apple.TextEdit PlainTextEncoding -int 4"
print::command "defaults write com.apple.TextEdit PlainTextEncodingForWrite -int 4"

# Enable the debug menu in Disk Utility
print::command "defaults write com.apple.DiskUtility DUDebugMenuEnabled -bool true"
print::command "defaults write com.apple.DiskUtility advanced-image-options -bool true"

# Auto-play videos when opened with QuickTime Player
print::command "defaults write com.apple.QuickTimePlayerX MGPlayMovieOnOpen -bool true"

###############################################################################
# Mac App Store                                                               #
###############################################################################

# Enable the WebKit Developer Tools in the Mac App Store
print::command "defaults write com.apple.appstore WebKitDeveloperExtras -bool true"

# Enable Debug Menu in the Mac App Store
print::command "defaults write com.apple.appstore ShowDebugMenu -bool true"

# Enable the automatic update check
print::command "defaults write com.apple.SoftwareUpdate AutomaticCheckEnabled -bool true"

# Check for software updates daily, not just once per week
print::command "defaults write com.apple.SoftwareUpdate ScheduleFrequency -int 1"

# Download newly available updates in background
print::command "defaults write com.apple.SoftwareUpdate AutomaticDownload -int 1"

# Install System data files & security updates
print::command "defaults write com.apple.SoftwareUpdate CriticalUpdateInstall -int 1"

# Automatically download apps purchased on other Macs
print::command "defaults write com.apple.SoftwareUpdate ConfigDataInstall -int 1"

# Turn on app auto-update
print::command "defaults write com.apple.commerce AutoUpdate -bool true"

# Allow the App Store to reboot machine on macOS updates
print::command "defaults write com.apple.commerce AutoUpdateRestartRequired -bool true"

###############################################################################
# Photos                                                                      #
###############################################################################

# Prevent Photos from opening automatically when devices are plugged in
print::command "defaults -currentHost write com.apple.ImageCapture disableHotPlug -bool true"

###############################################################################
# Messages                                                                    #
###############################################################################

# Disable automatic emoji substitution (i.e. use plain text smileys)
print::command "defaults write com.apple.messageshelper.MessageController SOInputLineSettings -dict-add \"automaticEmojiSubstitutionEnablediMessage\" -bool false"

# Disable smart quotes as it’s annoying for messages that contain code
print::command "defaults write com.apple.messageshelper.MessageController SOInputLineSettings -dict-add \"automaticQuoteSubstitutionEnabled\" -bool false"

# Disable continuous spell checking
print::command "defaults write com.apple.messageshelper.MessageController SOInputLineSettings -dict-add \"continuousSpellCheckingEnabled\" -bool false"

###############################################################################
# Google Chrome & Google Chrome Canary                                        #
###############################################################################

# Disable the all too sensitive backswipe on trackpads
print::command "defaults write com.google.Chrome AppleEnableSwipeNavigateWithScrolls -bool false"
print::command "defaults write com.google.Chrome.canary AppleEnableSwipeNavigateWithScrolls -bool false"

# Disable the all too sensitive backswipe on Magic Mouse
print::command "defaults write com.google.Chrome AppleEnableMouseSwipeNavigateWithScrolls -bool false"
print::command "defaults write com.google.Chrome.canary AppleEnableMouseSwipeNavigateWithScrolls -bool false"

# Use the system-native print preview dialog
print::command "defaults write com.google.Chrome DisablePrintPreview -bool true"
print::command "defaults write com.google.Chrome.canary DisablePrintPreview -bool true"

# Expand the print dialog by default
print::command "defaults write com.google.Chrome PMPrintingExpandedStateForPrint2 -bool true"
print::command "defaults write com.google.Chrome.canary PMPrintingExpandedStateForPrint2 -bool true"

###############################################################################
# Kill affected applications                                                  #
###############################################################################

for app in "Activity Monitor" \
	"Address Book" \
	"Calendar" \
	"cfprefsd" \
	"Contacts" \
	"Dock" \
	"Finder" \
	"Google Chrome Canary" \
	"Google Chrome" \
	"Mail" \
	"Messages" \
	"Opera" \
	"Photos" \
	"Safari" \
	"SizeUp" \
	"Spectacle" \
	"SystemUIServer" \
	"Terminal" \
	"Transmission" \
	"Tweetbot" \
	"Twitter" \
	"iCal"; do
	print::command "sudo killall "${app}" &> /dev/null"
done

print::info "Done. Note that some of these changes require a logout/restart to take effect."

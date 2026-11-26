// The window's icons: Blode's filled set, by the names the rest of the app uses for them.

// Use the individual exports so the package index is not evaluated at startup.
import ArrowExpandHorFilledIcon from 'blode-icons-react/icons/arrow-expand-hor-filled';
import ArrowLeftFilledIcon from 'blode-icons-react/icons/arrow-left-filled';
import ArrowRightFilledIcon from 'blode-icons-react/icons/arrow-right-filled';
import ArrowRotateClockwiseFilledIcon from 'blode-icons-react/icons/arrow-rotate-clockwise-filled';
import Bell2FilledIcon from 'blode-icons-react/icons/bell-2-filled';
import BrowserTabsFilledIcon from 'blode-icons-react/icons/browser-tabs-filled';
import CheckFilledIcon from 'blode-icons-react/icons/check-filled';
import ChevronBottomFilledIcon from 'blode-icons-react/icons/chevron-bottom-filled';
import ChevronLeftMediumFilledIcon from 'blode-icons-react/icons/chevron-left-medium-filled';
import ChevronTopFilledIcon from 'blode-icons-react/icons/chevron-top-filled';
import CircleRecordFilledIcon from 'blode-icons-react/icons/circle-record-filled';
import ClipboardFilledIcon from 'blode-icons-react/icons/clipboard-filled';
import CmdFilledIcon from 'blode-icons-react/icons/cmd-filled';
import ColorPaletteFilledIcon from 'blode-icons-react/icons/color-palette-filled';
import ConsoleFilledIcon from 'blode-icons-react/icons/console-filled';
import CrossMediumFilledIcon from 'blode-icons-react/icons/cross-medium-filled';
import DotGrid1x3HorizontalFilledIcon from 'blode-icons-react/icons/dot-grid-1x3-horizontal-filled';
import DotGrid2x3FilledIcon from 'blode-icons-react/icons/dot-grid-2x3-filled';
import Folder1FilledIcon from 'blode-icons-react/icons/folder-1-filled';
import GlobeFilledIcon from 'blode-icons-react/icons/globe-filled';
import HashtagFilledIcon from 'blode-icons-react/icons/hashtag-filled';
import Images1FilledIcon from 'blode-icons-react/icons/images-1-filled';
import KeyboardFilledIcon from 'blode-icons-react/icons/keyboard-filled';
import LayoutGrid1FilledIcon from 'blode-icons-react/icons/layout-grid-1-filled';
import MagicWandFilledIcon from 'blode-icons-react/icons/magic-wand-filled';
import MagnifyingGlassFilledIcon from 'blode-icons-react/icons/magnifying-glass-filled';
import PauseFilledIcon from 'blode-icons-react/icons/pause-filled';
import PlayFilledIcon from 'blode-icons-react/icons/play-filled';
import PlusMediumFilledIcon from 'blode-icons-react/icons/plus-medium-filled';
import SettingsGear1FilledIcon from 'blode-icons-react/icons/settings-gear-1-filled';
import SidebarSimpleLeftWideFilledIcon from 'blode-icons-react/icons/sidebar-simple-left-wide-filled';
import SplitFilledIcon from 'blode-icons-react/icons/split-filled';
import StopFilledIcon from 'blode-icons-react/icons/stop-filled';
import VideoFilledIcon from 'blode-icons-react/icons/video-filled';
import WindowFilledIcon from 'blode-icons-react/icons/window-filled';

export type IconName =
    | 'close' | 'plus' | 'split' | 'width' | 'grid' | 'search' | 'settings' | 'command' | 'terminal' | 'copy' | 'record' | 'stop'
    | 'chevron' | 'sidebar' | 'gear' | 'dots' | 'grip' | 'up' | 'down' | 'check' | 'folder' | 'bell' | 'image' | 'play' | 'pause' | 'quake'
    | 'link' | 'hash' | 'studio' | 'palette' | 'video' | 'back' | 'forward' | 'reload' | 'browser' | 'globe' | 'left' | 'keyboard';

const ICONS: Record<IconName, (props: { size?: number }) => any> = {
    close: CrossMediumFilledIcon,
    plus: PlusMediumFilledIcon,
    split: SplitFilledIcon,
    width: ArrowExpandHorFilledIcon,
    grid: LayoutGrid1FilledIcon,
    search: MagnifyingGlassFilledIcon,
    settings: SettingsGear1FilledIcon,
    gear: SettingsGear1FilledIcon,
    command: CmdFilledIcon,
    terminal: ConsoleFilledIcon,
    copy: ClipboardFilledIcon,
    record: CircleRecordFilledIcon,
    stop: StopFilledIcon,
    chevron: ChevronBottomFilledIcon,
    down: ChevronBottomFilledIcon,
    up: ChevronTopFilledIcon,
    sidebar: SidebarSimpleLeftWideFilledIcon,
    dots: DotGrid1x3HorizontalFilledIcon,
    grip: DotGrid2x3FilledIcon,
    check: CheckFilledIcon,
    folder: Folder1FilledIcon,
    bell: Bell2FilledIcon,
    image: Images1FilledIcon,
    play: PlayFilledIcon,
    pause: PauseFilledIcon,
    quake: WindowFilledIcon,
    link: HashtagFilledIcon,
    hash: HashtagFilledIcon,
    studio: MagicWandFilledIcon,
    palette: ColorPaletteFilledIcon,
    video: VideoFilledIcon,
    keyboard: KeyboardFilledIcon,
    back: ArrowLeftFilledIcon,
    forward: ArrowRightFilledIcon,
    reload: ArrowRotateClockwiseFilledIcon,
    browser: BrowserTabsFilledIcon,
    globe: GlobeFilledIcon,
    left: ChevronLeftMediumFilledIcon,
};

/** An icon in the color of the text it's in. `strokeWidth` is taken for the outline icons this replaced, and means nothing here. */
export function Icon(props: { name: IconName; size?: number; strokeWidth?: number }) {
    const Shape = ICONS[props.name];
    return <Shape size={props.size ?? 16} />;
}

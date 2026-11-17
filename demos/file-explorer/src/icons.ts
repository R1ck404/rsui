// The Blode icons this app uses, each from its own file, in their filled variants.
//
// `import { FolderIcon } from 'blode-icons-react'` is how the package is meant to be used, and it works here. But the
// package root re-exports all 6,116 icon modules, and loading them added about 470 ms to every start (590 ms against
// 120 ms for a two-icon test app). Importing each icon's own file keeps startup instant. The props are the same as
// lucide-react's: size, color, strokeWidth and absoluteStrokeWidth.

export { default as ArrowLeftIcon } from '../node_modules/blode-icons-react/dist/arrow-left-filled.js';
export { default as ArrowRightIcon } from '../node_modules/blode-icons-react/dist/arrow-right-filled.js';
export { default as ArrowUpIcon } from '../node_modules/blode-icons-react/dist/arrow-up-filled.js';
export { default as RefreshIcon } from '../node_modules/blode-icons-react/dist/arrow-rotate-clockwise-filled.js';
export { default as SearchIcon } from '../node_modules/blode-icons-react/dist/magnifying-glass-filled.js';
export { default as GridIcon } from '../node_modules/blode-icons-react/dist/layout-grid-1-filled.js';
export { default as ListIcon } from '../node_modules/blode-icons-react/dist/list-bullets-filled.js';
export { default as TableIcon } from '../node_modules/blode-icons-react/dist/table-filled.js';
export { default as SidebarIcon } from '../node_modules/blode-icons-react/dist/layout-sidebar-filled.js';
export { default as PreviewIcon } from '../node_modules/blode-icons-react/dist/sidebar-simple-right-wide-filled.js';
export { default as SettingsIcon } from '../node_modules/blode-icons-react/dist/settings-gear-1-filled.js';
export { default as EyeIcon } from '../node_modules/blode-icons-react/dist/eye-open-filled.js';
export { default as EyeOffIcon } from '../node_modules/blode-icons-react/dist/eye-slash-filled.js';
export { default as PlusIcon } from '../node_modules/blode-icons-react/dist/plus-medium-filled.js';
export { default as XIcon } from '../node_modules/blode-icons-react/dist/x-filled.js';
export { default as TrashIcon } from '../node_modules/blode-icons-react/dist/trash-can-filled.js';
export { default as CopyIcon } from '../node_modules/blode-icons-react/dist/files-filled.js';
export { default as CutIcon } from '../node_modules/blode-icons-react/dist/scissors-1-filled.js';
export { default as PasteIcon } from '../node_modules/blode-icons-react/dist/clipboard-filled.js';
export { default as PencilIcon } from '../node_modules/blode-icons-react/dist/pencil-filled.js';
export { default as SunIcon } from '../node_modules/blode-icons-react/dist/sun-filled.js';
export { default as MoonIcon } from '../node_modules/blode-icons-react/dist/moon-filled.js';
export { default as KeyboardIcon } from '../node_modules/blode-icons-react/dist/keyboard-filled.js';
export { default as SortIcon } from '../node_modules/blode-icons-react/dist/sort-arrow-up-down-filled.js';
export { default as CheckIcon } from '../node_modules/blode-icons-react/dist/check-filled.js';
export { default as InfoIcon } from '../node_modules/blode-icons-react/dist/circle-info-filled.js';
export { default as ChevronRightIcon } from '../node_modules/blode-icons-react/dist/chevron-right-filled.js';
export { default as ChevronDownIcon } from '../node_modules/blode-icons-react/dist/chevron-down-small-filled.js';
export { default as PaletteIcon } from '../node_modules/blode-icons-react/dist/color-palette-filled.js';
export { default as GaugeIcon } from '../node_modules/blode-icons-react/dist/gauge-filled.js';
export { default as BoltIcon } from '../node_modules/blode-icons-react/dist/lightning-filled.js';
export { default as TerminalIcon } from '../node_modules/blode-icons-react/dist/console-filled.js';
export { default as ExternalIcon } from '../node_modules/blode-icons-react/dist/arrow-out-of-box-filled.js';
export { default as SlidersIcon } from '../node_modules/blode-icons-react/dist/settings-slider-hor-filled.js';
export { default as LayoutIcon } from '../node_modules/blode-icons-react/dist/layout-dashboard-filled.js';
export { default as StarIcon } from '../node_modules/blode-icons-react/dist/star-filled.js';
export { default as PinIcon } from '../node_modules/blode-icons-react/dist/pin-filled.js';
export { default as HistoryIcon } from '../node_modules/blode-icons-react/dist/history-filled.js';
export { default as FoldersIcon } from '../node_modules/blode-icons-react/dist/folders-filled.js';

export { default as FolderIcon } from '../node_modules/blode-icons-react/dist/folder-1-filled.js';
export { default as FolderOpenIcon } from '../node_modules/blode-icons-react/dist/folder-open-filled.js';
export { default as FileIcon } from '../node_modules/blode-icons-react/dist/file-bend-filled.js';
export { default as FileTextIcon } from '../node_modules/blode-icons-react/dist/file-text-filled.js';
export { default as FilePdfIcon } from '../node_modules/blode-icons-react/dist/file-pdf-filled.js';
export { default as FileZipIcon } from '../node_modules/blode-icons-react/dist/file-zip-filled.js';
export { default as FileChartIcon } from '../node_modules/blode-icons-react/dist/file-chart-filled.js';
export { default as FileLinkIcon } from '../node_modules/blode-icons-react/dist/file-link-filled.js';
export { default as FileLockIcon } from '../node_modules/blode-icons-react/dist/file-lock-filled.js';
export { default as CodeIcon } from '../node_modules/blode-icons-react/dist/code-brackets-filled.js';
export { default as ImageIcon } from '../node_modules/blode-icons-react/dist/images-1-filled.js';
export { default as VideoIcon } from '../node_modules/blode-icons-react/dist/video-filled.js';
export { default as AudioIcon } from '../node_modules/blode-icons-react/dist/headphones-filled.js';
export { default as AppIcon } from '../node_modules/blode-icons-react/dist/window-app-filled.js';
export { default as DiskIcon } from '../node_modules/blode-icons-react/dist/floppy-disk-1-filled.js';
export { default as FontIcon } from '../node_modules/blode-icons-react/dist/text-size-filled.js';

export { default as HomeIcon } from '../node_modules/blode-icons-react/dist/home-filled.js';
export { default as DesktopIcon } from '../node_modules/blode-icons-react/dist/studio-display-filled.js';
export { default as DownloadIcon } from '../node_modules/blode-icons-react/dist/file-download-filled.js';
export { default as DocumentsIcon } from '../node_modules/blode-icons-react/dist/notes-filled.js';
export { default as DriveIcon } from '../node_modules/blode-icons-react/dist/server-filled.js';
export { default as UsbIcon } from '../node_modules/blode-icons-react/dist/usb-filled.js';
export { default as CloudIcon } from '../node_modules/blode-icons-react/dist/cloud-filled.js';
export { default as ComputerIcon } from '../node_modules/blode-icons-react/dist/computer-use-filled.js';

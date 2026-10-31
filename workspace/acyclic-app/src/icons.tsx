// The app's icons: Lucide's, in one size and stroke, always beside or inside a control that names itself, so each is
// hidden from assistive technology.

import type { LucideIcon } from "lucide-react";

export {
  ArrowUpLeft,
  Check,
  ChevronRight,
  CircleAlert,
  FolderInput,
  Inbox,
  LayoutDashboard,
  Link2,
  ListTree,
  Map as MapIcon,
  MoreHorizontal,
  Pencil,
  Plus,
  RotateCcw,
  Waypoints,
  X,
} from "lucide-react";

export function Icon({ of: Of, size = 18 }: { of: LucideIcon; size?: number }) {
  return <Of size={size} strokeWidth={1.75} aria-hidden="true" focusable="false" className="icon-svg" />;
}

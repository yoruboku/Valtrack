import type { PlayerRow as Player } from "../ipc/types";
import { pendingOf, type PartyMark } from "../lib/players";
import { ROW_GRID, ROW_PLATE, TEAM_TINT, type TableLayout, type TeamSide } from "../lib/table";
import { AgentCell } from "./cells/AgentCell";
import { NameCell } from "./cells/NameCell";
import { PeakCell, RankCell } from "./cells/RankCell";
import { SkinCell } from "./cells/SkinCell";
import {
  HeadshotCell,
  KdCell,
  ResultPips,
  RrChangeCell,
  WinRateCell,
  type StatsScope,
} from "./cells/StatCells";
import { Tooltip } from "./Tooltip";

/** Same colour for everyone in a party. Solo players keep the column empty. */
function PartyBar({ mark, inferred }: { mark: PartyMark | undefined; inferred?: boolean }) {
  if (!mark) return <span />;
  const tooltip = inferred
    ? `Estimated party of ${mark.size} (recent match teammates — players may not be queued together)`
    : `Party of ${mark.size}`;
  return (
    <Tooltip content={tooltip}>
      <span
        className={`mx-auto block h-6 w-0.5 rounded-full ${inferred ? "opacity-75" : ""}`}
        style={{
          backgroundColor: mark.color,
          outline: inferred ? "1px dashed rgba(255,255,255,0.4)" : "none",
        }}
      />
    </Tooltip>
  );
}

/** One player. Cells are rendered in the column order defined by `lib/table.ts`. */
export function PlayerRow({
  player,
  side,
  layout,
}: {
  player: Player;
  side: TeamSide;
  layout: TableLayout;
}) {
  const tint = TEAM_TINT[side];
  const pending = pendingOf(player);
  // `??` because snapshots captured before `statsMatches` existed lack the field.
  const scope: StatsScope = {
    matches: player.statsMatches ?? null,
    results: player.recentResults.length,
  };

  return (
    <div
      style={{ gridTemplateColumns: layout.template }}
      className={`group h-12 rounded-r-md bg-linear-to-r ${ROW_PLATE} transition-colors ${ROW_GRID} ${tint.row} ${
        player.isSelf ? "bg-white/[0.04] ring-1 ring-white/10 ring-inset" : ""
      }`}
    >
      <PartyBar
        mark={player.partyId ? layout.parties.get(player.partyId) : undefined}
        inferred={player.partyInferred}
      />
      <AgentCell
        agent={player.agent}
        group={player.group ?? null}
        selectionState={player.agentSelectionState}
        level={player.accountLevel}
      />
      <NameCell player={player} />
      <RankCell
        rank={player.currentRank}
        rr={player.rr}
        leaderboardRank={player.leaderboardRank}
        pending={pending.rank}
      />
      <PeakCell rank={player.peakRank} act={player.peakRankAct} pending={pending.rank} />
      <HeadshotCell
        percent={player.headshotPercent}
        scope={scope}
        pending={pending.recentStats}
      />
      <KdCell kd={player.kd} scope={scope} pending={pending.recentStats} />
      <WinRateCell winRate={player.winRate} pending={pending.rank} />
      <ResultPips results={player.recentResults} pending={pending.history} />
      <RrChangeCell change={player.rrChange} pending={pending.history} />
      {layout.withSkins && (
        <SkinCell skin={player.vandalSkin} weapon="Vandal" pending={pending.skins} />
      )}
      {layout.withSkins && (
        <SkinCell skin={player.phantomSkin} weapon="Phantom" pending={pending.skins} />
      )}
    </div>
  );
}

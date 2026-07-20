import { BaseEdge, type EdgeProps } from '@xyflow/react'

// The default bezier collapses into a barely visible squiggle when an edge
// points back to its own node (source handle right, target handle left).
// Route self-loops as a wide arc above the node instead, label at the apex.
export default function SelfLoopEdge({
  sourceX,
  sourceY,
  targetX,
  targetY,
  markerEnd,
  style,
  label,
  labelStyle,
  labelBgStyle,
}: EdgeProps) {
  const reachX = 56
  const reachY = 148
  const path =
    `M ${sourceX} ${sourceY} ` +
    `C ${sourceX + reachX} ${sourceY - reachY}, ` +
    `${targetX - reachX} ${targetY - reachY}, ` +
    `${targetX} ${targetY}`
  // Apex of the cubic sits at 3/4 of the control-point offset above the handles.
  const labelX = (sourceX + targetX) / 2
  const labelY = Math.min(sourceY, targetY) - reachY * 0.75

  return (
    <BaseEdge
      path={path}
      markerEnd={markerEnd}
      style={style}
      label={label}
      labelX={labelX}
      labelY={labelY}
      labelStyle={labelStyle}
      labelBgStyle={labelBgStyle}
      labelShowBg
    />
  )
}

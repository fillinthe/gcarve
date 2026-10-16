(misc reader cases: modal state, units, incremental, machine coords, work zero, dwell, errors)
%
N10 G90 G20 G17 G94 G54
M3 S18000
G0 X1 Y1 Z0.2
G1 Z-0.25 F50
G1 X3 F100
G2 X3 Y1 I1 J0
G3 X4 Y2 Z-0.3 I0.5 J0.5 ; helix-ish
G0X5Y1
g1x6 f80
G91
G1 X0.5 Y-0.25
G0 Z1
G90
G53 G0 Z0
G53 X10 Y10
G4 P1.5
G10 L20 P1 X0 Y0
G1 X1 Y1
G21
G1 X25.4 Y12.7 F254
G20
G2 X2 Y2 R1
G38.2 Z-1
M7
M98
Q5
G1 X1.2.3
X
G0 X-.5 Y+.25
M5
G0 X0 Y0
M30

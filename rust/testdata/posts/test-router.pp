+ Sample post
POST_NAME = "Test Router (inch)(*.tap)"
FILE_EXTENSION = "tap"
UNITS = "INCHES"
LINE_ENDING = "[13][10]"
VAR X_POSITION = [X|A|X|1.4]
VAR Y_POSITION = [Y|A|Y|1.4]
VAR Z_POSITION = [Z|A|Z|1.4]
VAR FEED_RATE = [F|A|F|1.1]
VAR SPINDLE_SPEED = [S|A|S|1.0]
begin HEADER
"( [TP_FILENAME] )"
"G90 G20"
"[T] M6"
"[S] M3"
end
begin RAPID_MOVE
"G0 [X] [Y] [Z]"
end
begin FEED_MOVE
"G1 [X] [Y] [Z] [F]"
end
begin CW_ARC_MOVE
"G2 [X] [Y] [I] [J] [F]"
end
begin CCW_ARC_MOVE
"G3 [X] [Y] [I] [J] [F]"
end
begin FOOTER
"M5"
"M30"
end

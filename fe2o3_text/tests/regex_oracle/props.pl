#!/usr/bin/env perl
# Writes props_expected.txt: Perl's own Unicode property data (Unicode::UCD), which
# tests/regex.rs compares with the tables fe2o3_text generates from the UCD.  Perl parses the UCD
# with its own code, so agreement is evidence about the generator and the lookups, not a
# restatement of them.  Perl's Unicode version is older than the tables', so the test compares only
# the code points Perl has assigned and reports what changed since.
#
# Run from this directory:  perl props.pl > props_expected.txt
use strict;
use warnings;
use Unicode::UCD qw(prop_invmap prop_invlist);

binmode STDOUT, ':utf8';
print "V\t", Unicode::UCD::UnicodeVersion(), "\n";

for my $p (['GC', 'General_Category'], ['SC', 'Script'], ['SCX', 'Script_Extensions']) {
	my ($tag, $name) = @$p;
	my ($list, $map) = prop_invmap($name);
	for my $i (0 .. $#$list) {
		my $v = $map->[$i];
		$v = join(' ', sort @$v) if ref $v;
		printf "%s\t%X\t%s\n", $tag, $list->[$i], $v;
	}
}

# Every binary property the tables carry, by the names the generator gives them.  One Perl does
# not know is listed as unknown rather than silently dropped.
my @bins = qw(
	ASCII_Hex_Digit Alphabetic Bidi_Control Case_Ignorable Cased Changes_When_Casefolded
	Changes_When_Casemapped Changes_When_Lowercased Changes_When_Titlecased
	Changes_When_Uppercased Dash Default_Ignorable_Code_Point Deprecated Diacritic Emoji
	Emoji_Component Emoji_Modifier Emoji_Modifier_Base Emoji_Presentation Extended_Pictographic
	Extender Grapheme_Base Grapheme_Extend Hex_Digit Hyphen IDS_Binary_Operator
	IDS_Trinary_Operator IDS_Unary_Operator ID_Compat_Math_Continue ID_Compat_Math_Start
	ID_Continue ID_Start Ideographic Join_Control Logical_Order_Exception Lowercase Math
	Modifier_Combining_Mark Noncharacter_Code_Point Pattern_Syntax Pattern_White_Space
	Prepended_Concatenation_Mark Quotation_Mark Radical Regional_Indicator Sentence_Terminal
	Soft_Dotted Terminal_Punctuation Unified_Ideograph Uppercase Variation_Selector White_Space
	XID_Continue XID_Start
);
for my $b (@bins) {
	my @inv = eval { prop_invlist($b) };
	if ($@ || !@inv) {
		print "BIN\t$b\tunknown\n";
		next;
	}
	print "BIN\t$b\t", join(' ', map { sprintf '%X', $_ } @inv), "\n";
}
